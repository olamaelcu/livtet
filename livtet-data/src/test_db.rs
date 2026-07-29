use camino_tempfile::Utf8TempDir as TempDir;
use miette::IntoDiagnostic;
use sqlx::sqlite::SqlitePool;

use crate::{
    migrator::{Kind, connect_with_migrations},
    state::SharedState,
};

pub struct TestDb {
    pub pool: SqlitePool,
    temp_dir: TempDir,
}

impl TestDb {
    pub async fn new(kinds: Option<Vec<Kind>>) -> Result<Self, sqlx::Error> {
        let kinds = kinds.unwrap_or_else(|| vec![Kind::Business]);
        let temp_dir = TempDir::new()
            .into_diagnostic()
            .map_err(|e| sqlx::Error::AnyDriverError(e.into()))?;
        let db_file = temp_dir.path().join("test.db");
        let db_path = db_file.as_os_str().to_string_lossy().to_string();
        std::fs::write(&db_file, []).unwrap();
        let pool = connect_with_migrations(&db_path, kinds).await?;

        Ok(Self { pool, temp_dir })
    }

    #[cfg_attr(test, mutants::skip)]
    pub fn path(&self) -> String {
        self.temp_dir.path().to_string()
    }

    pub fn state(&self) -> SharedState {
        let db_path = self
            .temp_dir
            .path()
            .join("test.db")
            .as_os_str()
            .to_string_lossy()
            .into_owned();
        SharedState {
            pool: self.pool.clone(),
            db_path,
        }
    }
}
