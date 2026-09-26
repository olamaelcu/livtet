use livtet_types::DbId;
use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize, DeriveEntityModel)]
#[sea_orm(table_name = "editions")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: DbId,
    pub work_id: DbId,
    pub group_id: Option<DbId>,
    pub title: Option<String>,
    pub published_date: Option<time::Date>,
    pub format_id: Option<DbId>,
    pub language_id: Option<DbId>,
    pub notes: Option<String>,
    pub description: Option<String>,
    /// Optional per-edition format metadata (e.g. audiobook duration and
    /// chapters), validated against the edition format's
    /// `FormatMetadataSchema` at the application layer.
    pub format_metadata: Option<serde_json::Value>,
    pub created_at: time::PrimitiveDateTime,
    pub updated_at: Option<time::PrimitiveDateTime>,
}

#[cfg_attr(feature = "fake", derive(fake::Dummy))]
#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[sea_orm(
        belongs_to = "super::works::Entity",
        from = "Column::WorkId",
        to = "super::works::Column::Id"
    )]
    Work,
    #[sea_orm(
        belongs_to = "super::edition_groups::Entity",
        from = "Column::GroupId",
        to = "super::edition_groups::Column::Id"
    )]
    EditionGroup,
}

impl Related<super::works::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Work.def()
    }
}

impl ActiveModelBehavior for ActiveModel {}

// Manual `Dummy` impl: `fake` has no `Dummy` for `serde_json::Value`, so the
// JSON column fakes as null (mirrors `formats.rs` / `reading_sessions.rs`).
#[cfg(feature = "fake")]
impl fake::Dummy<fake::Faker> for Model {
    fn dummy_with_rng<R: fake::RngExt + ?Sized>(faker: &fake::Faker, rng: &mut R) -> Self {
        use fake::Fake;
        Model {
            id: faker.fake_with_rng(rng),
            work_id: faker.fake_with_rng(rng),
            group_id: faker.fake_with_rng(rng),
            title: faker.fake_with_rng(rng),
            published_date: faker.fake_with_rng(rng),
            format_id: faker.fake_with_rng(rng),
            language_id: faker.fake_with_rng(rng),
            notes: faker.fake_with_rng(rng),
            description: faker.fake_with_rng(rng),
            format_metadata: Some(serde_json::Value::Null),
            created_at: faker.fake_with_rng(rng),
            updated_at: faker.fake_with_rng(rng),
        }
    }
}
