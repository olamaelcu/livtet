use livtet_types::DbId;
use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize, DeriveEntityModel)]
#[sea_orm(table_name = "reading_sessions")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: DbId,
    pub edition_id: DbId,
    pub format_id: DbId,
    pub source_id: Option<DbId>,
    pub started_at: time::PrimitiveDateTime,
    pub ended_at: Option<time::PrimitiveDateTime>,
    pub duration_seconds: Option<i64>,
    pub raw_progression: Option<serde_json::Value>,
    pub progress_delta: f64,
    pub last_location: Option<String>,
    pub notes: Option<String>,
    pub created_at: time::PrimitiveDateTime,
    pub updated_at: Option<time::PrimitiveDateTime>,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[sea_orm(
        belongs_to = "super::editions::Entity",
        from = "Column::EditionId",
        to = "super::editions::Column::Id"
    )]
    Edition,
    #[sea_orm(
        belongs_to = "super::formats::Entity",
        from = "Column::FormatId",
        to = "super::formats::Column::Id"
    )]
    Format,
    #[sea_orm(
        belongs_to = "super::reading_sources::Entity",
        from = "Column::SourceId",
        to = "super::reading_sources::Column::Id"
    )]
    Source,
}

impl ActiveModelBehavior for ActiveModel {}

#[cfg(feature = "fake")]
impl fake::Dummy<fake::Faker> for Model {
    fn dummy_with_rng<R: fake::RngExt + ?Sized>(faker: &fake::Faker, rng: &mut R) -> Self {
        use fake::Fake;
        Model {
            id: faker.fake_with_rng(rng),
            edition_id: faker.fake_with_rng(rng),
            format_id: faker.fake_with_rng(rng),
            source_id: faker.fake_with_rng(rng),
            started_at: faker.fake_with_rng(rng),
            ended_at: faker.fake_with_rng(rng),
            duration_seconds: faker.fake_with_rng(rng),
            raw_progression: Some(serde_json::Value::Null),
            progress_delta: faker.fake_with_rng(rng),
            last_location: faker.fake_with_rng(rng),
            notes: faker.fake_with_rng(rng),
            created_at: faker.fake_with_rng(rng),
            updated_at: faker.fake_with_rng(rng),
        }
    }
}
