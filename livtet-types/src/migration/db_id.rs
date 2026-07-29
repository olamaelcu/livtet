pub fn pk_db_id(col: &str) -> String {
    format!("`{col}` BINARY(16) NOT NULL PRIMARY KEY")
}

pub fn db_id(col: &str) -> String {
    format!("`{col}` BINARY(16) NOT NULL")
}

pub fn db_id_null(col: &str) -> String {
    format!("`{col}` BINARY(16) NULL")
}
