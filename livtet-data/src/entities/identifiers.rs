use livtet_types::DbId;
use sea_orm::entity::prelude::*;

#[cfg_attr(feature = "fake", derive(fake::Dummy))]
#[derive(Clone, Debug, DeriveEntityModel)]
#[sea_orm(table_name = "identifiers")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: DbId,
    /// URN value, e.g. "urn:isbn:978-0-06-112008-4", "urn:wikidata:Q193359"
    #[sea_orm(unique)]
    pub value: String,
    /// URN scheme ("isbn", "oclc", "lccn", "doi", "openlibrary",
    /// "wikidata", "custom"). New code goes through `IdentifierKind`
    /// in `livtet-core` to keep the values in sync.
    pub kind: String,
}

#[cfg_attr(feature = "fake", derive(fake::Dummy))]
#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
