use livtet_types::DbId;
use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, DeriveEntityModel)]
#[sea_orm(table_name = "formats")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: DbId,
    pub name: String,
    pub metadata_schema: serde_json::Value,
    pub progress_unit: Option<String>,
}

#[cfg_attr(feature = "fake", derive(fake::Dummy))]
#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}

#[cfg(feature = "fake")]
impl fake::Dummy<fake::Faker> for Model {
    fn dummy_with_rng<R: fake::RngExt + ?Sized>(faker: &fake::Faker, rng: &mut R) -> Self {
        use fake::Fake;
        Model {
            id: faker.fake_with_rng(rng),
            name: faker.fake_with_rng(rng),
            metadata_schema: serde_json::Value::Null,
            progress_unit: None,
        }
    }
}
