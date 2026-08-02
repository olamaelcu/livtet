use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::task::AbortHandle;
use tracing::{debug, error, warn};

use crate::error::PluginResult;
use crate::host_manager::{PluginHostManager, SharedEventEmitter};
use crate::watch::WatchResult;

const WATCH_CURSOR_KEY: &str = "watch.cursor";

struct WatchState {
    handle: Option<AbortHandle>,
    cursor: Option<String>,
    interval: Duration,
}

impl WatchState {
    fn new(interval: Duration, cursor: Option<String>) -> Self {
        Self {
            handle: None,
            cursor,
            interval,
        }
    }
}

pub struct WatchScheduler {
    emitter: SharedEventEmitter,
    watchers: Arc<Mutex<HashMap<String, WatchState>>>,
}

impl WatchScheduler {
    pub fn new(emitter: SharedEventEmitter) -> Self {
        Self {
            emitter,
            watchers: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub fn start(
        &self,
        manager: Arc<Mutex<PluginHostManager>>,
        plugin_id: &str,
        interval: Duration,
        start_cursor: Option<String>,
    ) {
        let id = plugin_id.to_string();
        let mut watchers = self.watchers.lock().expect("watch scheduler lock");
        if watchers.contains_key(&id) {
            debug!(%plugin_id, "watch already running");
            return;
        }
        let state = WatchState::new(interval, start_cursor);
        watchers.insert(id.clone(), state);
        drop(watchers);

        let watchers = Arc::clone(&self.watchers);
        let emitter = Arc::clone(&self.emitter);

        let handle = tokio::spawn(Self::watch_loop(
            manager,
            watchers,
            emitter,
            id.clone(),
        ));

        let mut watchers = self.watchers.lock().expect("watch scheduler lock");
        if let Some(state) = watchers.get_mut(&id) {
            state.handle = Some(handle.abort_handle());
        }
    }

    pub fn stop(&self, plugin_id: &str) {
        let mut watchers = self.watchers.lock().expect("watch scheduler lock");
        if let Some(state) = watchers.remove(plugin_id) {
            debug!(%plugin_id, "watch stopped");
            if let Some(handle) = state.handle {
                handle.abort();
            }
        }
    }

    pub fn is_watching(&self, plugin_id: &str) -> bool {
        self.watchers
            .lock()
            .expect("watch scheduler lock")
            .contains_key(plugin_id)
    }

    pub fn cursor_for(&self, plugin_id: &str) -> Option<String> {
        self.watchers
            .lock()
            .expect("watch scheduler lock")
            .get(plugin_id)
            .and_then(|s| s.cursor.clone())
    }

    async fn watch_loop(
        manager: Arc<Mutex<PluginHostManager>>,
        watchers: Arc<Mutex<HashMap<String, WatchState>>>,
        emitter: SharedEventEmitter,
        plugin_id: String,
    ) {
        loop {
            let interval = {
                let watchers = watchers.lock().expect("watch scheduler lock");
                match watchers.get(&plugin_id) {
                    Some(s) => s.interval,
                    None => {
                        debug!(%plugin_id, "watch state removed; stopping loop");
                        return;
                    }
                }
            };

            tokio::time::sleep(interval).await;

            let since = {
                watchers
                    .lock()
                    .expect("watch scheduler lock")
                    .get(&plugin_id)
                    .and_then(|s| s.cursor.clone())
            };

            let result: PluginResult<WatchResult> = {
                let manager = Arc::clone(&manager);
                let plugin_id = plugin_id.clone();
                tokio::task::spawn_blocking(move || {
                    tokio::runtime::Handle::current().block_on(async {
                        let mut mgr = manager.lock().expect("watch scheduler manager lock");
                        mgr.call_watch(&plugin_id, since).await
                    })
                })
                .await
                .expect("spawn_blocking watch call")
            };

            match result {
                Ok(watch_result) => {
                    let has_changes = !watch_result.changes.is_empty();
                    let next_cursor = watch_result.next_cursor.clone();

                    {
                        let mut watchers = watchers.lock().expect("watch scheduler lock");
                        if let Some(state) = watchers.get_mut(&plugin_id) {
                            state.cursor = next_cursor.clone();
                        }
                    }

                    if let Some(ref cursor) = next_cursor {
                        let manager = Arc::clone(&manager);
                        let pid = plugin_id.clone();
                        let cursor = cursor.clone();
                        let persist_result: PluginResult<()> = tokio::task::spawn_blocking(
                            move || {
                                tokio::runtime::Handle::current().block_on(async {
                                    let mgr =
                                        manager.lock().expect("watch scheduler manager lock");
                                    mgr.write_setting_direct(&pid, WATCH_CURSOR_KEY, &cursor)
                                        .await
                                })
                            },
                        )
                        .await
                        .expect("spawn_blocking persist cursor");
                        if let Err(e) = persist_result {
                            warn!(%plugin_id, error = %e, "failed to persist watch cursor");
                        }
                    }

                    if has_changes {
                        let payload = serde_json::json!({
                            "plugin_id": &plugin_id,
                            "changes": watch_result.changes,
                            "has_more": watch_result.has_more,
                            "next_cursor": watch_result.next_cursor,
                        });
                        let name = format!("plugin:watch:{plugin_id}");
                        emitter.emit(&name, payload);
                    }
                }
                Err(e) => {
                    error!(%plugin_id, error = %e, "watch poll failed");
                }
            }
        }
    }
}
