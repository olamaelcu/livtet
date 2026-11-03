//! Contributor API (methods on [`LivtetStore`]).
//!
//! The schema calls the entity `authors` and attaches it to editions
//! through `edition_authors`. A contributor is a name shared across the
//! library — an entry exists once and every edition it worked on points
//! at it — so deleting a link never deletes the name, and the role lives
//! on the link rather than on the person.
//!
//! `authors` carries no unique constraint on the name, so a caller that
//! wants to avoid duplicates asks [`LivtetStore::find_contributors`]
//! first; the catalog hydration path does exactly that.

use livtet_core::data::entities::{authors, edition_authors};
use livtet_core::data::orm::{
    ActiveModelTrait, ColumnTrait, EntityTrait, QueryFilter, QueryOrder, Set,
};
use livtet_core::types::DbId;

use crate::dto::{Contribution, Contributor};
use crate::error::LivtetError;
use crate::mutations::require_edition;
use crate::store::LivtetStore;

#[uniffi::export(async_runtime = "tokio")]
impl LivtetStore {
    /// Create a contributor. Names are trimmed and must not be empty.
    pub async fn create_contributor(&self, name: String) -> Result<Contributor, LivtetError> {
        let name = clean_name(name)?;
        let db = self.state.db_conn();

        let model = authors::ActiveModel {
            id: Set(DbId::new()),
            name: Set(name),
        }
        .insert(&db)
        .await?;

        Ok(Contributor {
            id: model.id,
            name: model.name,
        })
    }

    /// One contributor, or `None` when the id is unknown.
    pub async fn get_contributor(&self, id: DbId) -> Result<Option<Contributor>, LivtetError> {
        let db = self.state.db_conn();
        Ok(authors::Entity::find_by_id(id)
            .one(&db)
            .await?
            .map(|m| Contributor {
                id: m.id,
                name: m.name,
            }))
    }

    /// Rename a contributor. Every edition they are attached to picks the
    /// name up, because editions hold a reference rather than a copy.
    pub async fn update_contributor(
        &self,
        id: DbId,
        name: String,
    ) -> Result<Contributor, LivtetError> {
        let name = clean_name(name)?;
        let db = self.state.db_conn();

        let model = require_contributor(&db, id).await?;
        let mut active: authors::ActiveModel = model.into();
        active.name = Set(name);
        let updated = active.update(&db).await?;

        Ok(Contributor {
            id: updated.id,
            name: updated.name,
        })
    }

    /// Delete a contributor and, by cascade, every edition link to them.
    /// Returns `false` when the id is unknown.
    pub async fn delete_contributor(&self, id: DbId) -> Result<bool, LivtetError> {
        let db = self.state.db_conn();
        Ok(authors::Entity::delete_by_id(id)
            .exec(&db)
            .await?
            .rows_affected
            > 0)
    }

    /// Contributors whose name contains `name`, case-insensitively, ordered
    /// by name. The dedupe lookup before creating one.
    pub async fn find_contributors(&self, name: String) -> Result<Vec<Contributor>, LivtetError> {
        let db = self.state.db_conn();
        let needle = name.trim().to_lowercase();

        let models = authors::Entity::find()
            .order_by_asc(authors::Column::Name)
            .all(&db)
            .await?;

        Ok(models
            .into_iter()
            .filter(|m| needle.is_empty() || m.name.to_lowercase().contains(&needle))
            .map(|m| Contributor {
                id: m.id,
                name: m.name,
            })
            .collect())
    }

    /// Contributors attached to one edition, ordered by name then role.
    pub async fn list_edition_contributors(
        &self,
        edition_id: DbId,
    ) -> Result<Vec<Contribution>, LivtetError> {
        let db = self.state.db_conn();

        let links = edition_authors::Entity::find()
            .filter(edition_authors::Column::EditionId.eq(edition_id))
            .all(&db)
            .await?;
        if links.is_empty() {
            return Ok(Vec::new());
        }

        let names: std::collections::HashMap<DbId, String> = authors::Entity::find()
            .filter(
                authors::Column::Id.is_in(links.iter().map(|l| l.author_id).collect::<Vec<_>>()),
            )
            .all(&db)
            .await?
            .into_iter()
            .map(|m| (m.id, m.name))
            .collect();

        let mut out: Vec<Contribution> = links
            .into_iter()
            .map(|link| Contribution {
                edition_id: link.edition_id,
                contributor_id: link.author_id,
                name: names.get(&link.author_id).cloned().unwrap_or_default(),
                role: link.role,
            })
            .collect();
        out.sort_by(|a, b| a.name.cmp(&b.name).then_with(|| a.role.cmp(&b.role)));
        Ok(out)
    }

    /// Attach a contributor to an edition in `role`. Idempotent: the same
    /// (edition, contributor, role) triple is reused rather than doubled.
    pub async fn add_edition_contributor(
        &self,
        edition_id: DbId,
        contributor_id: DbId,
        role: String,
    ) -> Result<Contribution, LivtetError> {
        let role = clean_role(role)?;
        let db = self.state.db_conn();
        require_edition(&db, edition_id).await?;
        let author = require_contributor(&db, contributor_id).await?;

        attach_contributor(&db, edition_id, contributor_id, &role).await?;
        self.sync_edition_index(edition_id).await?;

        Ok(Contribution {
            edition_id,
            contributor_id,
            name: author.name,
            role,
        })
    }

    /// Move a contributor from one role to another on the same edition.
    ///
    /// The role is part of the link's primary key, so this detaches
    /// `from_role` and attaches `to_role`; other roles the contributor
    /// holds on that edition are untouched.
    pub async fn update_edition_contributor(
        &self,
        edition_id: DbId,
        contributor_id: DbId,
        from_role: String,
        to_role: String,
    ) -> Result<Contribution, LivtetError> {
        let from_role = clean_role(from_role)?;
        let to_role = clean_role(to_role)?;
        let db = self.state.db_conn();
        require_edition(&db, edition_id).await?;
        let author = require_contributor(&db, contributor_id).await?;

        detach_contributor(&db, edition_id, contributor_id, &from_role).await?;
        attach_contributor(&db, edition_id, contributor_id, &to_role).await?;
        self.sync_edition_index(edition_id).await?;

        Ok(Contribution {
            edition_id,
            contributor_id,
            name: author.name,
            role: to_role,
        })
    }

    /// Detach a contributor from an edition in one role, returning whether
    /// the link existed. The contributor row itself is kept: other
    /// editions still point at it.
    pub async fn remove_edition_contributor(
        &self,
        edition_id: DbId,
        contributor_id: DbId,
        role: String,
    ) -> Result<bool, LivtetError> {
        let role = clean_role(role)?;
        let db = self.state.db_conn();

        let removed = detach_contributor(&db, edition_id, contributor_id, &role).await?;
        if removed {
            self.sync_edition_index(edition_id).await?;
        }
        Ok(removed)
    }
}

fn clean_name(name: String) -> Result<String, LivtetError> {
    let name = name.trim().to_string();
    if name.is_empty() {
        return Err(LivtetError::InvalidInput(
            "contributor name must not be empty".to_string(),
        ));
    }
    Ok(name)
}

fn clean_role(role: String) -> Result<String, LivtetError> {
    let role = role.trim().to_lowercase();
    if role.is_empty() {
        return Err(LivtetError::InvalidInput(
            "contributor role must not be empty".to_string(),
        ));
    }
    Ok(role)
}

async fn require_contributor(
    db: &livtet_core::data::orm::DatabaseConnection,
    id: DbId,
) -> Result<authors::Model, LivtetError> {
    authors::Entity::find_by_id(id)
        .one(db)
        .await?
        .ok_or_else(|| LivtetError::NotFound {
            entity: "authors".to_string(),
            id: id.to_string(),
        })
}

async fn attach_contributor(
    db: &livtet_core::data::orm::DatabaseConnection,
    edition_id: DbId,
    contributor_id: DbId,
    role: &str,
) -> Result<(), LivtetError> {
    let existing = edition_authors::Entity::find()
        .filter(edition_authors::Column::EditionId.eq(edition_id))
        .filter(edition_authors::Column::AuthorId.eq(contributor_id))
        .filter(edition_authors::Column::Role.eq(role))
        .one(db)
        .await?;
    if existing.is_none() {
        edition_authors::Entity::insert(edition_authors::ActiveModel {
            edition_id: Set(edition_id),
            author_id: Set(contributor_id),
            role: Set(role.to_string()),
        })
        .exec(db)
        .await?;
    }
    Ok(())
}

async fn detach_contributor(
    db: &livtet_core::data::orm::DatabaseConnection,
    edition_id: DbId,
    contributor_id: DbId,
    role: &str,
) -> Result<bool, LivtetError> {
    let affected = edition_authors::Entity::delete_many()
        .filter(edition_authors::Column::EditionId.eq(edition_id))
        .filter(edition_authors::Column::AuthorId.eq(contributor_id))
        .filter(edition_authors::Column::Role.eq(role))
        .exec(db)
        .await?
        .rows_affected;
    Ok(affected > 0)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;

    async fn store_with_edition() -> (camino_tempfile::Utf8TempDir, Arc<LivtetStore>, DbId) {
        let tmp = camino_tempfile::tempdir().unwrap();
        let store = LivtetStore::open(
            tmp.path().join("livtet.db").to_string(),
            tmp.path().join("index").to_string(),
        )
        .await
        .expect("open store");

        livtet_core::data::seed::seed_database(
            &store.state.db_conn(),
            &livtet_core::data::seed::SeedConfig {
                num_works: 2,
                ..Default::default()
            },
        )
        .await
        .expect("seed");

        let work = &store.list_works(1, 0, None, None, None).await.unwrap()[0];
        let edition = store.list_editions(work.id).await.unwrap()[0].id;
        (tmp, store, edition)
    }

    #[tokio::test]
    async fn contributor_crud_roundtrip() {
        let (_tmp, store, _edition) = store_with_edition().await;

        let created = store
            .create_contributor("  Wole Soyinka  ".to_string())
            .await
            .unwrap();
        assert_eq!(created.name, "Wole Soyinka", "name is trimmed");

        let fetched = store.get_contributor(created.id).await.unwrap().unwrap();
        assert_eq!(fetched.id, created.id);
        assert_eq!(fetched.name, created.name);

        let renamed = store
            .update_contributor(created.id, "Wole Soyinka (1934–)".to_string())
            .await
            .unwrap();
        assert_eq!(renamed.name, "Wole Soyinka (1934–)");

        let found = store
            .find_contributors("soyinka".to_string())
            .await
            .unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].id, created.id);

        assert!(store.delete_contributor(created.id).await.unwrap());
        assert!(!store.delete_contributor(created.id).await.unwrap());
        assert!(store.get_contributor(created.id).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn blank_names_and_roles_are_rejected() {
        let (_tmp, store, edition) = store_with_edition().await;

        let err = store
            .create_contributor("   ".to_string())
            .await
            .unwrap_err();
        assert!(matches!(err, LivtetError::InvalidInput(_)), "{err:?}");

        let who = store
            .create_contributor("Chinua Achebe".to_string())
            .await
            .unwrap();
        let err = store
            .add_edition_contributor(edition, who.id, "  ".to_string())
            .await
            .unwrap_err();
        assert!(matches!(err, LivtetError::InvalidInput(_)), "{err:?}");
    }

    #[tokio::test]
    async fn attach_move_and_detach_contributor_roles() {
        let (_tmp, store, edition) = store_with_edition().await;
        let who = store
            .create_contributor("Chinua Achebe".to_string())
            .await
            .unwrap();

        let author = store
            .add_edition_contributor(edition, who.id, "Author".to_string())
            .await
            .unwrap();
        assert_eq!(author.role, "author", "role is normalised");
        assert_eq!(author.name, "Chinua Achebe");

        // Same triple twice stays one link.
        store
            .add_edition_contributor(edition, who.id, "author".to_string())
            .await
            .unwrap();

        let translator = store
            .add_edition_contributor(edition, who.id, "translator".to_string())
            .await
            .unwrap();
        assert_eq!(translator.role, "translator");

        let listed = store.list_edition_contributors(edition).await.unwrap();
        // The seeded edition already carries its own contributors, so only
        // this contributor's links are counted.
        let mine = |role: &str| {
            listed
                .iter()
                .filter(|c| c.contributor_id == who.id && c.role == role)
                .count()
        };
        assert_eq!(mine("author"), 1, "one row per role");
        assert_eq!(mine("translator"), 1);

        let moved = store
            .update_edition_contributor(
                edition,
                who.id,
                "translator".to_string(),
                "editor".to_string(),
            )
            .await
            .unwrap();
        assert_eq!(moved.role, "editor");
        let roles: Vec<String> = store
            .list_edition_contributors(edition)
            .await
            .unwrap()
            .into_iter()
            .filter(|c| c.contributor_id == who.id)
            .map(|c| c.role)
            .collect();
        // Ordered by name, then role: same contributor, so role decides.
        assert_eq!(roles, vec!["author".to_string(), "editor".to_string()]);

        assert!(
            store
                .remove_edition_contributor(edition, who.id, "author".to_string())
                .await
                .unwrap()
        );
        assert!(
            !store
                .remove_edition_contributor(edition, who.id, "author".to_string())
                .await
                .unwrap()
        );

        // The contributor row survives losing a link.
        assert!(store.get_contributor(who.id).await.unwrap().is_some());
        let remaining: Vec<String> = store
            .list_edition_contributors(edition)
            .await
            .unwrap()
            .into_iter()
            .filter(|c| c.contributor_id == who.id)
            .map(|c| c.role)
            .collect();
        assert_eq!(remaining, vec!["editor".to_string()]);
    }

    #[tokio::test]
    async fn contributor_is_shared_across_editions() {
        let (_tmp, store, edition) = store_with_edition().await;
        let other_work = &store.list_works(10, 0, None, None, None).await.unwrap()[1];
        let other_edition = store.list_editions(other_work.id).await.unwrap()[0].id;

        let who = store
            .create_contributor("Ngũgĩ wa Thiong'o".to_string())
            .await
            .unwrap();
        store
            .add_edition_contributor(edition, who.id, "author".to_string())
            .await
            .unwrap();
        store
            .add_edition_contributor(other_edition, who.id, "author".to_string())
            .await
            .unwrap();

        // One row, two references: a rename reaches both editions.
        store
            .update_contributor(who.id, "Ngugi wa Thiong'o".to_string())
            .await
            .unwrap();
        for ed in [edition, other_edition] {
            let listed: Vec<_> = store
                .list_edition_contributors(ed)
                .await
                .unwrap()
                .into_iter()
                .filter(|c| c.contributor_id == who.id)
                .collect();
            assert_eq!(listed.len(), 1);
            assert_eq!(listed[0].name, "Ngugi wa Thiong'o");
        }
    }

    #[tokio::test]
    async fn attaching_to_a_missing_edition_or_contributor_fails() {
        let (_tmp, store, edition) = store_with_edition().await;
        let who = store
            .create_contributor("Someone".to_string())
            .await
            .unwrap();

        let err = store
            .add_edition_contributor(DbId::new(), who.id, "author".to_string())
            .await
            .unwrap_err();
        assert!(matches!(err, LivtetError::NotFound { .. }), "{err:?}");

        let err = store
            .add_edition_contributor(edition, DbId::new(), "author".to_string())
            .await
            .unwrap_err();
        assert!(matches!(err, LivtetError::NotFound { .. }), "{err:?}");
    }
}
