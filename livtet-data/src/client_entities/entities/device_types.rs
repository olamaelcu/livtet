use livtet_types::DbId;
use sea_orm::entity::prelude::*;

#[cfg_attr(feature = "fake", derive(fake::Dummy))]
#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "device_types")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: DbId,
    pub name: String,
    pub value: i32,
    pub created_at: time::PrimitiveDateTime,
    pub updated_at: Option<time::PrimitiveDateTime>,
}

#[cfg_attr(feature = "fake", derive(fake::Dummy))]
#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}

impl Entity {
    /// Resolve a device_type_id FK to a display name.
    ///
    /// Fast path: if the ULID's random component matches a known seed
    /// constant (Desktop=400, Mobile=401, Web=402, E-Reader=403) return
    /// the canonical name without a query. Slow path: query `device_types`
    /// by ID, which returns the user-specific name (e.g. "KOReader on Kobo
    /// Libra 2") for non-canonical variants seeded via `pair_device`.
    /// Returns `DbErr::RecordNotFound` if no row exists.
    pub async fn display_name_for(db: &DbConn, fk: DbId) -> Result<String, DbErr> {
        if let Some(name) = match fk.0.random() {
            400 => Some("Desktop"),
            401 => Some("Mobile"),
            402 => Some("Web"),
            403 => Some("E-Reader"),
            _ => None,
        } {
            return Ok(name.to_string());
        }
        Self::find_by_id(fk)
            .one(db)
            .await?
            .map(|m| m.name)
            .ok_or_else(|| DbErr::RecordNotFound(format!("device_types row not found for id {fk}")))
    }
}
