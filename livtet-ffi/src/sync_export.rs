use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use livtet_sync::client::SyncClient;

use crate::runtime;

static CANCEL_FLAGS: once_cell::sync::Lazy<std::sync::Mutex<Vec<(String, Arc<AtomicBool>)>>> =
    once_cell::sync::Lazy::new(Default::default);

#[derive(uniffi::Record)]
pub struct SyncConfig {
    pub host: String,
    pub port: u16,
    pub device_id: String,
    pub device_name: String,
    pub session_token: String,
}

#[derive(uniffi::Enum)]
pub enum SyncState {
    Idle,
    Syncing {
        progress: f64,
    },
    Completed {
        timestamp: u64,
        books_added: u32,
        books_updated: u32,
    },
    Failed {
        error: String,
    },
}

fn get_db_conn() -> Result<livtet_database::orm::DatabaseConnection, crate::MobileError> {
    let state = crate::get_state()?;
    Ok(state.db_conn())
}

fn base_url(config: &SyncConfig) -> String {
    format!("http://{}:{}", config.host, config.port)
}

#[uniffi::export]
pub fn pair_with_desktop(config: SyncConfig) -> Result<(), crate::MobileError> {
    runtime::block_on(async {
        let db = get_db_conn()?;
        let mut client = SyncClient::new(&db, &config.device_id);
        client
            .connect(&base_url(&config))
            .await
            .map_err(|e| crate::MobileError::Network(e.to_string()))
    })
}

#[uniffi::export]
pub fn sync_once(config: SyncConfig) -> SyncState {
    runtime::block_on(async {
        let db = match get_db_conn() {
            Ok(db) => db,
            Err(e) => {
                return SyncState::Failed {
                    error: e.to_string(),
                };
            }
        };

        let cancel = Arc::new(AtomicBool::new(false));
        {
            let mut flags = CANCEL_FLAGS.lock().unwrap();
            flags.push((config.device_id.clone(), cancel.clone()));
        }

        let mut client = SyncClient::new(&db, &config.device_id);
        if let Err(e) = client.connect(&base_url(&config)).await {
            return SyncState::Failed {
                error: format!("connect failed: {e}"),
            };
        }

        if cancel.load(Ordering::Relaxed) {
            let _ = client.engine();
            return SyncState::Idle;
        }

        let remote = match client.pull_since(0, 1000).await {
            Ok(r) => r,
            Err(e) => {
                return SyncState::Failed {
                    error: format!("pull failed: {e}"),
                };
            }
        };

        if cancel.load(Ordering::Relaxed) {
            return SyncState::Idle;
        }

        let local_changes = match client.engine().pull_changes(0, 1000).await {
            Ok(r) => r,
            Err(e) => {
                return SyncState::Failed {
                    error: format!("local pull failed: {e}"),
                };
            }
        };

        if cancel.load(Ordering::Relaxed) {
            return SyncState::Idle;
        }

        if !local_changes.changes.is_empty() {
            if let Err(e) = client.push(local_changes.changes).await {
                return SyncState::Failed {
                    error: format!("push failed: {e}"),
                };
            }
        }

        SyncState::Completed {
            timestamp: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs(),
            books_added: remote
                .changes
                .iter()
                .filter(|c| c.operation == "INSERT" && c.entity_type == "work")
                .count() as u32,
            books_updated: remote
                .changes
                .iter()
                .filter(|c| c.operation != "INSERT" && c.entity_type == "work")
                .count() as u32,
        }
    })
}

#[uniffi::export]
pub fn cancel_sync(device_id: String) {
    let mut flags = CANCEL_FLAGS.lock().unwrap();
    flags.retain(|(id, flag)| {
        if id == &device_id {
            flag.store(true, Ordering::Relaxed);
            false
        } else {
            true
        }
    });
}
