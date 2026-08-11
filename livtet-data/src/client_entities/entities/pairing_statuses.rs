use sea_orm::DbErr;

crate::vocab_table!("pairing_statuses", {
    /// Resolve a status_id FK to a display name.
    ///
    /// Fast path: if the ULID's random component matches a known seed
    /// constant (Pending=500, Approved=501, Rejected=502) return the
    /// name without a query. Slow path: query `pairing_statuses` by ID.
    /// Returns `DbErr::RecordNotFound` if no row exists.
    pub async fn display_name_for(db: &DbConn, fk: livtet_types::DbId) -> Result<String, DbErr> {
        if let Some(name) = match fk.0.random() {
            500 => Some("Pending"),
            501 => Some("Approved"),
            502 => Some("Rejected"),
            _ => None,
        } {
            return Ok(name.to_string());
        }
        Self::find_by_id(fk)
            .one(db)
            .await?
            .map(|m| m.name)
            .ok_or_else(|| {
                DbErr::RecordNotFound(format!("pairing_statuses row not found for id {fk}"))
            })
    }
});