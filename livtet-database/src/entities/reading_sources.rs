use livtet_types::DbId;
use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize, DeriveEntityModel)]
#[sea_orm(table_name = "reading_sources")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: DbId,
    pub urn: String,
    pub name: String,
    pub emoji: Option<String>,
    pub color: Option<String>,
    pub attributes: Option<serde_json::Value>,
    pub plugin_id: Option<String>,
    pub deleted_at: Option<time::PrimitiveDateTime>,
    pub created_at: time::PrimitiveDateTime,
    pub updated_at: Option<time::PrimitiveDateTime>,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}

#[cfg(feature = "fake")]
impl fake::Dummy<fake::Faker> for Model {
    fn dummy_with_rng<R: fake::RngExt + ?Sized>(faker: &fake::Faker, rng: &mut R) -> Self {
        use fake::Fake;
        Model {
            id: faker.fake_with_rng(rng),
            urn: faker.fake_with_rng(rng),
            name: faker.fake_with_rng(rng),
            emoji: faker.fake_with_rng(rng),
            color: faker.fake_with_rng(rng),
            attributes: Some(serde_json::Value::Null),
            plugin_id: faker.fake_with_rng(rng),
            deleted_at: faker.fake_with_rng(rng),
            created_at: faker.fake_with_rng(rng),
            updated_at: faker.fake_with_rng(rng),
        }
    }
}
