use std::{
    collections::{HashMap, HashSet},
    convert::TryFrom,
    sync::Arc,
    time::Duration,
};

use aes_gcm::{
    Aes256Gcm, Nonce,
    aead::{Aead, KeyInit},
};
use camino::{Utf8Path, Utf8PathBuf};
use fs_err as fs;
use livtet_data::sql::{self, AssertSqlSafe, SqlitePool};
use rand::{Rng as _, rng};
use tokio::{
    io::AsyncWriteExt,
    process::{Child, ChildStdin, ChildStdout, Command},
    sync::{Mutex, oneshot},
};
use tracing::{error, info, warn};

/// Shared message-dispatch loop for host operations that need to process
/// interleaved messages (logs, HTTP, dispatchable requests) while waiting
/// for a terminal response. The success arms are pasted verbatim into the
/// match; the macro only generates the three invariant arms (Log,
/// HttpRequest, dispatch).
macro_rules! recv_loop {
    ($self:expr, [$($arm:tt)+], $dispatch:pat, $log_prefix:expr) => {
        loop {
            let msg: HostToMain = $self.recv().await?;
            match msg {
                $($arm)+
                HostToMain::Log { plugin_id, level, message } => {
                    forward_log(&plugin_id, &level, &message);
                }
                // TBD: the HttpRequest host bridge was removed
                // alongside `livtet_core::crud`. Restore a
                // generic IPC channel that routes HTTP requests
                // through the host's outbound HTTP client.
                HostToMain::HttpRequest { .. } => {
                    warn!("HttpRequest dropped: host bridge removed");
                }
                msg @ $dispatch => {
                    if let Err(e) = $self.dispatch(msg).await {
                        warn!(error = %e, concat!("dispatch failed during ", $log_prefix));
                    }
                }
                other => {
                    warn!(message = ?other, concat!("unexpected message ", $log_prefix));
                }
            }
        }
    };
}

/// Decrypt the content of a plugin-secrets file. Returns `None` if the
/// file is missing, corrupt, or the HMAC key doesn't match — the caller
/// decides how to surface the error.
fn decrypt_secrets_content(content: &str, hmac_key: &[u8; 32]) -> Option<HashMap<String, String>> {
    let decoded =
        base64::Engine::decode(&base64::engine::general_purpose::STANDARD, content.trim()).ok()?;
    if decoded.len() < 12 {
        return None;
    }
    let cipher = Aes256Gcm::new_from_slice(hmac_key.as_slice()).ok()?;
    let nonce = Nonce::try_from(&decoded[..12]).ok()?;
    cipher
        .decrypt(&nonce, &decoded[12..])
        .ok()
        .and_then(|p| String::from_utf8(p).ok())
        .and_then(|j| serde_json::from_str(&j).ok())
}

use crate::{
    discovery::{DiscoveredPlugin, PluginSource, scan_plugins},
    error::{PluginError, PluginResult},
    keys::TrustStore,
    link_resolver::{ResolveLinksOptions, ResolveLinksResult},
    manifest::PluginManifest,
    plugin_requires::PluginRequires,
    protocol::{HostToMain, MainToHost, MainToHostCallback},
    repository::hmac::HmacKey,
    system_secrets::PluginSystemSecret,
};

const LOAD_STATE_LOADED: &str = "loaded";
const LOAD_STATE_DISCOVERED: &str = "discovered";

/// Environment overrides applied to the spawned sidecar process.
///
/// Today this only carries the `LUA_PATH` / `LUA_CPATH` strings
/// that the parent computed from `luarocks::build_env(app_data_dir)`.
/// The Tauri-side wiring in `crates/livtet-tauri/` is responsible
/// for installing the rocks and computing these strings before
/// spawning the sidecar; this manager just hands them through to
/// `tokio::process::Command::env`. `None` fields mean "leave the
/// child's env untouched" (i.e. do not call `command.env(...)`).
#[derive(Default, Clone, Debug)]
pub struct CommandEnv {
    pub lua_path: Option<String>,
    pub lua_cpath: Option<String>,
}

/// Configuration bundle for spawning a sidecar host.
///
/// Bundles the cross-cutting concerns (HTTP client + Lua env) so the
/// low-level [`HostManager::spawn_with_db_emitter_log_dir`]
/// signature stays at six arguments. Callers that don't care about
/// Lua use [`HostSpawnConfig::default`].
#[derive(Clone, Default)]
pub struct HostSpawnConfig {
    pub command_env: CommandEnv,
}

struct PluginVersion {
    manifest: PluginManifest,
    source: String,
    load_state: String,
    missing_optional: Vec<String>,
}

struct LoadedPlugin {
    versions: HashMap<String, PluginVersion>,
    active_version: Option<String>,
}

struct PendingCall {
    #[allow(dead_code)]
    sender: oneshot::Sender<PluginResult<serde_json::Value>>,
}

/// Sink for Tauri-style event emissions.
///
/// The dispatcher in `livtet-plugins` doesn't depend on
/// `tauri` directly (the plugin crate stays portable for
/// non-Tauri harnesses), so the Tauri-side wiring installs a
/// trait object that knows how to turn a `(name, payload)`
/// pair into a `tauri::Emitter::emit(...)` call. The trait
/// object is `Send + Sync` because the dispatcher hands it
/// off to whichever async task happens to be running.
///
/// Bodies ignore the result of the emit; emit failures are
/// logged at the Tauri wiring layer (see e.g. the existing
/// `let _ = app.emit(...)` call sites in `livtet-tauri`).
pub trait EventEmitter: Send + Sync {
    fn emit(&self, name: &str, payload: serde_json::Value);
}

pub type SharedEventEmitter = Arc<dyn EventEmitter>;

/// No-op emitter used by callers that haven't installed a
/// Tauri-side handler yet (legacy spawn, unit tests, the
/// `host_lua` binary on its own). Logs nothing, does nothing.
pub struct NullEventEmitter;
impl EventEmitter for NullEventEmitter {
    fn emit(&self, _name: &str, _payload: serde_json::Value) {}
}

pub struct PluginHostManager {
    stdin: Arc<Mutex<ChildStdin>>,
    stdout: ChildStdout,
    child: Child,
    loaded_plugins: HashMap<String, LoadedPlugin>,
    pending_calls: HashMap<String, PendingCall>,
    runtime: String,
    /// Pool used to answer host-side requests for library data
    /// (`host.resolve_identifier`, `host.get_edition_info`,
    /// etc.). `None` for callers that don't need DB-backed
    /// answers — the request handlers return an error instead of
    /// hanging the host function on a SQL query.
    db: Option<SqlitePool>,
    /// Plugin ids the user has disabled. Applies to both
    /// bundled and disk-installed plugins. Persisted to
    /// `installed.json` by the Tauri command layer; the manager
    /// keeps the in-memory mirror and filters `list_plugins()`
    /// accordingly. Disabled plugins stay in `loaded_plugins`
    /// so re-enabling immediately restores their discoverable
    /// state.
    disabled: HashSet<String>,
    /// Plugin directory path. Used to compute the data directory
    /// for the secret fallback file when the OS keyring is unavailable.
    plugin_dir: Utf8PathBuf,
    /// HMAC key used by secret handlers. Loaded (or created) once
    /// at startup via `HmacKey::load_or_create_in_keyring()`. The
    /// struct guarantees the key is always present — handlers read
    /// `self.hmac_key.clone()` and never fall back to a Static key.
    hmac_key: Arc<HmacKey>,
    /// Trust store consulted during `register_discovered`. Disk-installed
    /// plugins (`PluginSource::Folder` / `LegacyFile`) aren't checked
    /// at registration time — they go through the full archive
    /// install pipeline where `verify_archive()` enforces trust.
    trust_store: TrustStore,
    /// Host-owned system secrets keyed by canonical variant.
    /// Populated at startup from the project's SOPS bundle (see ADR
    /// 0032) and surfaced read-only via `host.get_system_secret`.
    system_secrets: HashMap<PluginSystemSecret, String>,
    /// Optional OAuth handler. Set by the Tauri main process after
    /// constructing the `OAuthBroker`. When `None`, OAuth messages
    /// fall through to the `other` arm and reply with "unhandled".
    oauth_handler: Option<Arc<dyn crate::host_trait::OAuthDispatchHandler>>,
}

/// Build the [`tokio::process::Command`] used to spawn the plugin host
/// sidecar.
///
/// The host's clap CLI only declares the optional `--log-dir` long flag.
/// Passing any positional argument causes the host to exit 2 before
/// writing the IPC `Ready` handshake, which the parent surfaces as
/// `ipc error: read len: early eof`. The `build_command_tests` module
/// pins the argv to exactly `[binary_path]` (plus the `LIVTET_LOG_DIR`
/// env when `log_dir` is `Some`) so a future refactor cannot silently
/// re-introduce a positional arg.
///
/// `command_env` carries the per-spawn `LUA_PATH` / `LUA_CPATH`
/// overrides used to expose LuaRocks-installed modules to the
/// sidecar's `host.require`. Passing `None` for either field leaves
/// the corresponding env var untouched (so e.g. tests that don't
/// care about Lua can pass `CommandEnv::default()`).
fn build_command(
    binary: &Utf8Path,
    log_dir: Option<&Utf8Path>,
    command_env: &CommandEnv,
) -> Command {
    let mut command = Command::new(binary);
    command
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true);
    if let Some(dir) = log_dir {
        command.env("LIVTET_LOG_DIR", dir.as_std_path());
    }
    if let Some(p) = command_env.lua_path.as_deref() {
        command.env("LUA_PATH", p);
    }
    if let Some(p) = command_env.lua_cpath.as_deref() {
        command.env("LUA_CPATH", p);
    }
    command
}

/// Returns `true` when the embedded bundled plugin signer key is
/// present in `trust_store` (as either a builtin or user key).
///
/// Bundled plugin source bytes are only loaded if this returns
/// `true` — see `PluginHostManager::register_discovered`. The
/// "embed-time verification" name is a bit misleading because no
/// signature is checked at runtime: both the key and the plugin
/// bytes are compiled into the same binary, so an attacker who
/// swapped one would have swapped the other. The gate still
impl PluginHostManager {
    pub async fn spawn(
        binary: &Utf8Path,
        plugin_dir: Utf8PathBuf,
        hmac_key: Arc<HmacKey>,
    ) -> PluginResult<Self> {
        Self::spawn_with_db(binary, plugin_dir, None, hmac_key).await
    }

    /// Like [`Self::spawn`] but also takes a `SqlitePool` for
    /// answering host-side requests that need library data
    /// (`host.resolve_identifier`, `host.get_edition_info`, etc.)
    /// and a trait-object event sink for emitting
    /// `"reading-progress-updated"` (and future) Tauri events.
    /// Pass `None` for either argument to fall back to the
    /// legacy "no DB / no event bus" path — the handlers
    /// return an error instead of hanging the host.
    pub async fn spawn_with_db(
        binary: &Utf8Path,
        plugin_dir: Utf8PathBuf,
        db: Option<SqlitePool>,
        hmac_key: Arc<HmacKey>,
    ) -> PluginResult<Self> {
        Self::spawn_with_db_and_emitter(binary, plugin_dir, db, hmac_key).await
    }

    /// Like [`Self::spawn_with_db`] but also installs the
    /// `event_emitter` sink. The Tauri side calls this with a
    /// closure that turns `(name, payload)` into
    /// `app.emit(name, payload)`.
    ///
    /// `log_dir`, when `Some`, is exported to the child process via
    /// the `LIVTET_LOG_DIR` environment variable so the sidecar can
    /// resolve its log file location without needing a CLI argument.
    /// `None` leaves resolution to the sidecar's own logic (env var
    /// from a different source, or the bundle-ID `logs_dir()`
    /// fallback).
    pub async fn spawn_with_db_and_emitter(
        binary: &Utf8Path,
        plugin_dir: Utf8PathBuf,
        db: Option<SqlitePool>,
        hmac_key: Arc<HmacKey>,
    ) -> PluginResult<Self> {
        Self::spawn_with_db_emitter_log_dir(
            binary,
            plugin_dir,
            db,
            None,
            hmac_key,
            HostSpawnConfig::default(),
        )
        .await
    }

    /// Lowest-level spawn that also accepts a `log_dir` to be
    /// exported as `LIVTET_LOG_DIR`. Existing callers continue to
    /// use [`spawn_with_db_and_emitter`], which passes `None` here.
    /// The Tauri parent calls this directly with the canonical
    /// `livtet_core::paths::logs_dir()` so the sidecar writes to the
    /// same directory as the parent's main log file.
    pub async fn spawn_with_db_emitter_log_dir(
        binary: &Utf8Path,
        plugin_dir: Utf8PathBuf,
        db: Option<SqlitePool>,
        log_dir: Option<Utf8PathBuf>,
        hmac_key: Arc<HmacKey>,
        config: HostSpawnConfig,
    ) -> PluginResult<Self> {
        let mut command = build_command(binary, log_dir.as_deref(), &config.command_env);

        let mut child = command
            .spawn()
            .map_err(|e| PluginError::HostCrashed(format!("failed to spawn host binary: {e}")))?;

        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| PluginError::HostCrashed("host stdin not available".into()))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| PluginError::HostCrashed("host stdout not available".into()))?;

        let mut manager = Self {
            stdin: Arc::new(Mutex::new(stdin)),
            stdout,
            child,
            loaded_plugins: HashMap::new(),
            pending_calls: HashMap::new(),
            runtime: String::new(),
            db,
            disabled: HashSet::new(),
            plugin_dir: plugin_dir.clone(),
            hmac_key,
            system_secrets: HashMap::new(),
            trust_store: TrustStore::empty(),
            oauth_handler: None,
        };

        manager.wait_for_ready().await?;
        manager.discover(&plugin_dir).await?;
        Ok(manager)
    }

    /// Builder helper: register host-owned system secrets before
    /// any plugin asks for them. Call this immediately after
    /// `spawn` and before the first plugin load. Empty values
    /// are silently dropped.
    pub fn with_system_secrets(
        &mut self,
        system_secrets: HashMap<PluginSystemSecret, String>,
    ) -> &mut Self {
        self.system_secrets.clear();
        for (k, v) in system_secrets {
            if !v.is_empty() {
                self.system_secrets.insert(k, v);
            }
        }
        self
    }

    /// Builder helper: install the trust store consulted by
    /// `register_discovered` to gate bundled plugin loading.
    ///
    /// Call after `spawn_with_db_emitter_log_dir` and before
    /// `discover()`. In practice, Tauri-side callers should construct
    /// the store via `TrustStore::load_from_dir(...)` and pass it
    /// here so user-trusted keys are layered on top of the bundled
    /// `keys::bundled_trusted_keys()` set.
    pub fn with_trust_store(&mut self, trust_store: TrustStore) -> &mut Self {
        self.trust_store = trust_store;
        self
    }

    /// Builder helper: install the `OAuthDispatchHandler` that handles
    /// `OAuthRedeemRequest` / `OAuthTokenLookupRequest` /
    /// `OAuthRevokeRequest` IPC messages. The Tauri main process provides
    /// the concrete broker; the host side sees only the trait.
    pub fn with_oauth_handler(
        &mut self,
        handler: Arc<dyn crate::host_trait::OAuthDispatchHandler>,
    ) -> &mut Self {
        self.oauth_handler = Some(handler);
        self
    }

    async fn wait_for_ready(&mut self) -> PluginResult<()> {
        let msg: HostToMain = self.recv().await?;
        match msg {
            HostToMain::Ready { runtime } => {
                self.runtime = runtime;
                Ok(())
            }
            other => Err(PluginError::HostCrashed(format!(
                "expected Ready, got {other:?}"
            ))),
        }
    }

    async fn discover(&mut self, plugin_dir: &Utf8PathBuf) -> PluginResult<()> {
        // First-party plugins compiled into the binary (when the
        // `bundled` feature is enabled). These are added first so
        // User-installed disk plugins from the providers/ directory.
        let plugins = scan_plugins(plugin_dir)?;
        for plugin in plugins {
            self.register_discovered(plugin);
        }

        Ok(())
    }

    fn register_discovered(&mut self, plugin: DiscoveredPlugin) {
        let id = plugin.id.clone();
        let version = plugin.manifest.plugin.version.clone();
        let entry_path = plugin.manifest.plugin.entry.clone();

        let source = match plugin.source {
            PluginSource::Folder => {
                let path = plugin.path.join(&entry_path);
                match fs_err::read_to_string(&path) {
                    Ok(src) => src,
                    Err(e) => {
                        warn!(plugin = %id, version = %version, path = %path, "failed to read entry: {e}");
                        String::new()
                    }
                }
            }
            PluginSource::LegacyFile => match fs_err::read_to_string(&plugin.path) {
                Ok(src) => src,
                Err(e) => {
                    warn!(plugin = %id, version = %version, path = %plugin.path, "failed to read legacy file: {e}");
                    String::new()
                }
            },
        };
        let entry = self.loaded_plugins.entry(id).or_insert(LoadedPlugin {
            versions: HashMap::new(),
            active_version: None,
        });
        entry.versions.insert(
            version,
            PluginVersion {
                manifest: plugin.manifest,
                source,
                load_state: LOAD_STATE_DISCOVERED.to_string(),
                missing_optional: Vec::new(),
            },
        );
    }

    /// Check that a plugin declaring `requires.filesystem = true` has a
    /// Check that a plugin declaring `requires.filesystem = true` has a
    /// grant sidecar with non-empty `write_paths`. Logs a `tracing::warn!`
    /// if not. The plugin still loads — the warning is intended for
    /// operators reading plugin-host logs. The UI surface that calls
    /// out a missing grant is a follow-up in Sibling Spec 3.
    fn check_filesystem_grant(plugin_id: &str, grant: Option<&crate::permissions::ResolvedGrant>) {
        if let Some(grant) = grant {
            if grant.raw.write_paths.is_empty() {
                tracing::warn!(
                    "plugin {plugin_id:?} declares requires.filesystem=true \
                     but grant sidecar has empty write_paths; \
                     import_items will fail at runtime. \
                     Add write_paths to .../permissions/{plugin_id}.toml"
                );
            }
        } else {
            tracing::warn!(
                "plugin {plugin_id:?} declares requires.filesystem=true \
                 but no grant sidecar found; import_items will fail at runtime"
            );
        }
    }

    pub async fn load_plugin(&mut self, id: &str, version: &str) -> PluginResult<()> {
        // Check if plugin is already loaded - idempotency
        if let Some(entry) = self.loaded_plugins.get(id)
            && let Some(pv) = entry.versions.get(version)
            && pv.load_state == LOAD_STATE_LOADED
        {
            tracing::info!("load_plugin: plugin {id}@{version} already loaded");
            return Ok(());
        }

        let (manifest, source) = {
            let entry = self
                .loaded_plugins
                .get(id)
                .ok_or_else(|| PluginError::PluginNotFound(id.to_string()))?;
            let pv = entry
                .versions
                .get(version)
                .ok_or_else(|| PluginError::PluginNotFound(id.to_string()))?;
            (pv.manifest.clone(), pv.source.clone())
        };

        // If the manifest declares `requires.filesystem`, surface a
        // warning when the user's grant sidecar is missing or has no
        // `write_paths`. The plugin still loads — the warning is for
        // the plugin list UI to surface, since `import_items` will
        // fail at runtime without a write grant.
        if manifest
            .plugin
            .requires
            .get(&PluginRequires::Filesystem)
            .copied()
            .unwrap_or(false)
        {
            let perms_dir = crate::permissions::permissions_dir();
            match crate::permissions::load_grant(id, &perms_dir) {
                Ok(Some(grant)) => {
                    Self::check_filesystem_grant(id, Some(&grant));
                }
                Ok(None) => {
                    Self::check_filesystem_grant(id, None);
                }
                Err(e) => {
                    tracing::warn!(
                        plugin = %id,
                        error = %e,
                        "failed to load grant sidecar; cannot validate filesystem grant"
                    );
                }
            }
        }

        let manifest_json = serde_json::to_value(&manifest).map_err(|e| {
            PluginError::Serialization(format!("failed to serialize manifest: {e}"))
        })?;

        // Query pre-existing settings from the DB so the host
        // process can populate its in-memory map without a
        // round-trip per read. Returns `None` if no DB pool is
        // available; the host uses an empty map in that case.
        let settings = self.load_plugin_settings(id).await;

        // Forward the manifest's `rocks` list verbatim. The host
        // process consults this list (in a follow-up) to
        // pre-register expected rocks with the Lua loader so it
        // can produce friendlier error messages when
        // `host.require` is called for a rock the parent
        // promised to install via luarocks.
        let rocks = manifest.plugin.rocks.clone();

        self.send_main(&MainToHost::LoadPlugin {
            plugin_id: id.to_string(),
            manifest: manifest_json,
            source,
            data_dir: None,
            settings,
            rocks,
        })
        .await?;

        recv_loop!(self,
            [
                HostToMain::PluginLoaded { plugin_id, load_state, missing_optional } if plugin_id == id => {
                    if let Some(entry) = self.loaded_plugins.get_mut(id)
                        && let Some(pv) = entry.versions.get_mut(version)
                    {
                        pv.load_state = load_state;
                        pv.missing_optional = missing_optional;
                        entry.active_version = Some(version.to_string());
                    }
                    return Ok(());
                }
                HostToMain::PluginLoadError { plugin_id, error } if plugin_id == id => {
                    return Err(PluginError::PluginLoadFailed { id: id.to_string(), error });
                }
            ],
            HostToMain::ResolveIdentifierRequest { .. }
            | HostToMain::ResolveIdentifiersRequest { .. }
            | HostToMain::GetEditionInfoRequest { .. }
            | HostToMain::GetEditionIdentifiersRequest { .. }
            | HostToMain::FetchProgressRequest { .. }
            | HostToMain::UpsertProgressRequest { .. }
            | HostToMain::SecretRequest { .. }
            | HostToMain::SetSecretRequest { .. }
            | HostToMain::SetSettingRequest { .. },
            "while loading plugin"
        );
    }
    pub async fn unload_plugin(&mut self, id: &str, version: &str) -> PluginResult<()> {
        {
            let entry = self
                .loaded_plugins
                .get(id)
                .ok_or_else(|| PluginError::PluginNotFound(id.to_string()))?;
            if !entry.versions.contains_key(version) {
                return Err(PluginError::PluginNotFound(id.to_string()));
            }
        }

        self.send_main(&MainToHost::UnloadPlugin {
            plugin_id: id.to_string(),
        })
        .await?;

        recv_loop!(self,
            [
                HostToMain::PluginUnloaded { plugin_id } if plugin_id == id => {
                    if let Some(entry) = self.loaded_plugins.get_mut(id)
                        && let Some(pv) = entry.versions.get_mut(version)
                    {
                        pv.load_state = LOAD_STATE_DISCOVERED.to_string();
                        pv.missing_optional.clear();
                        if entry.active_version.as_deref() == Some(version) {
                            entry.active_version = None;
                        }
                    }
                    return Ok(());
                }
            ],
            HostToMain::ResolveIdentifierRequest { .. }
            | HostToMain::ResolveIdentifiersRequest { .. }
            | HostToMain::GetEditionInfoRequest { .. }
            | HostToMain::GetEditionIdentifiersRequest { .. }
            | HostToMain::FetchProgressRequest { .. }
            | HostToMain::UpsertProgressRequest { .. }
            | HostToMain::SecretRequest { .. }
            | HostToMain::SetSecretRequest { .. }
            | HostToMain::SetSettingRequest { .. },
            "while unloading plugin"
        );
    }

    pub async fn call(
        &mut self,
        plugin_id: &str,
        capability: &str,
        args: Vec<serde_json::Value>,
    ) -> PluginResult<serde_json::Value> {
        {
            let entry = self
                .loaded_plugins
                .get(plugin_id)
                .ok_or_else(|| PluginError::PluginNotFound(plugin_id.to_string()))?;
            let active = entry
                .active_version
                .as_deref()
                .ok_or_else(|| PluginError::PluginNotFound(plugin_id.to_string()))?;
            let pv = entry
                .versions
                .get(active)
                .ok_or_else(|| PluginError::PluginNotFound(plugin_id.to_string()))?;
            if pv.load_state != LOAD_STATE_LOADED {
                return Err(PluginError::PluginNotFound(plugin_id.to_string()));
            }
        }

        let call_id = ulid::Ulid::new().to_string();
        let (tx, _rx) = oneshot::channel();
        self.pending_calls
            .insert(call_id.clone(), PendingCall { sender: tx });

        self.send_main(&MainToHost::Call {
            id: call_id.clone(),
            plugin_id: plugin_id.to_string(),
            capability: capability.to_string(),
            args,
        })
        .await?;

        recv_loop!(self,
            [
                HostToMain::CallResult { id, ok, value, error } if id == call_id => {
                    let _ = self.pending_calls.remove(&call_id);
                    if ok {
                        return Ok(value.unwrap_or(serde_json::Value::Null));
                    }
                    let err = error.unwrap_or_else(|| {
                        "plugin call returned ok=false with no error string".to_string()
                    });
                    error!(plugin = %plugin_id, capability = %capability, "call error: {err}");
                    return Err(PluginError::Ipc(format!("call error (plugin={plugin_id}): {err}")));
                }
            ],
            HostToMain::SecretRequest { .. }
            | HostToMain::SetSecretRequest { .. }
            | HostToMain::ResolveIdentifierRequest { .. }
            | HostToMain::ResolveIdentifiersRequest { .. }
            | HostToMain::GetEditionInfoRequest { .. }
            | HostToMain::GetEditionIdentifiersRequest { .. }
            | HostToMain::FetchProgressRequest { .. }
            | HostToMain::UpsertProgressRequest { .. }
            | HostToMain::SetSettingRequest { .. },
            "during call"
        );
    }

    pub async fn resolve_links(
        &mut self,
        plugin_id: &str,
        urn: &str,
        options: ResolveLinksOptions,
    ) -> PluginResult<ResolveLinksResult> {
        let options_json = serde_json::to_value(&options)
            .map_err(|e| PluginError::Serialization(e.to_string()))?;
        let args = vec![serde_json::Value::String(urn.to_string()), options_json];
        let result = self.call(plugin_id, "resolve_links", args).await?;
        serde_json::from_value(result)
            .map_err(|e| PluginError::Serialization(format!("invalid link resolver response: {e}")))
    }

    /// Dispatch the `catalog_resolver` capability. Extracts
    /// bibliographic metadata from a library catalog URL.
    /// Returns the plugin's raw JSON value, or `None` if the
    /// plugin cannot resolve this URL. The Tauri command layer
    /// deserializes the JSON into a `PluginHit`.
    pub async fn resolve_catalog_url(
        &mut self,
        plugin_id: &str,
        url: &str,
    ) -> PluginResult<Option<serde_json::Value>> {
        let args = vec![serde_json::Value::String(url.to_string())];
        let result = self.call(plugin_id, "resolve_catalog_url", args).await?;
        if result.is_null() {
            return Ok(None);
        }
        Ok(Some(result))
    }

    /// Dispatch the `reading_progress` capability: list the import
    /// sources a plugin exposes (e.g. "KOReader Kosync", "Kobo
    /// Sync"). Returns the raw JSON value the plugin sent so callers
    /// can inspect `config_fields` and the like without re-parsing.
    pub async fn call_progress_sources(
        &mut self,
        plugin_id: &str,
    ) -> PluginResult<serde_json::Value> {
        self.call(plugin_id, "progress_sources", Vec::new()).await
    }

    /// Dispatch the `reading_progress` capability: fetch entries
    /// from a specific source. Returns the raw JSON value the
    /// plugin sent (the typed shape is in
    /// `crate::reading_progress::FetchProgressResult` for callers
    /// that want a typed decode).
    pub async fn call_fetch_progress(
        &mut self,
        plugin_id: &str,
        source_id: &str,
        config: serde_json::Value,
    ) -> PluginResult<serde_json::Value> {
        let args = vec![serde_json::Value::String(source_id.to_string()), config];
        self.call(plugin_id, "fetch_progress", args).await
    }

    /// Dispatch the `annotations` capability: list the import
    /// sources a plugin exposes (e.g. "Kindle Clippings File").
    /// Returns the raw JSON value the plugin sent so callers can
    /// inspect `config_fields` and the like without re-parsing.
    pub async fn call_annotation_sources(
        &mut self,
        plugin_id: &str,
    ) -> PluginResult<serde_json::Value> {
        self.call(plugin_id, "annotation_sources", Vec::new()).await
    }

    /// Dispatch the `annotations` capability: fetch entries from a
    /// specific source. Returns the raw JSON value the plugin sent
    /// (the typed shape is in
    /// `crate::annotations::FetchAnnotationsResult` for callers
    /// that want a typed decode).
    pub async fn call_fetch_annotations(
        &mut self,
        plugin_id: &str,
        source_id: &str,
        config: serde_json::Value,
    ) -> PluginResult<serde_json::Value> {
        let args = vec![serde_json::Value::String(source_id.to_string()), config];
        self.call(plugin_id, "fetch_annotations", args).await
    }

    /// Dispatch the `reading_list` capability: list the import
    /// sources a plugin exposes (e.g. "GoodReads Shelves").
    /// Returns the raw JSON value the plugin sent so callers can
    /// inspect `config_fields` and the like without re-parsing.
    pub async fn call_list_sources(&mut self, plugin_id: &str) -> PluginResult<serde_json::Value> {
        self.call(plugin_id, "list_sources", Vec::new()).await
    }

    /// Dispatch the `reading_list` capability: fetch lists and
    /// their items from a specific source. Returns the raw JSON
    /// value the plugin sent (the typed shape is in
    /// `crate::reading_list::FetchListsResult` for callers that
    /// want a typed decode).
    pub async fn call_fetch_lists(
        &mut self,
        plugin_id: &str,
        source_id: &str,
        config: serde_json::Value,
    ) -> PluginResult<serde_json::Value> {
        let args = vec![serde_json::Value::String(source_id.to_string()), config];
        self.call(plugin_id, "fetch_lists", args).await
    }

    /// Dispatch the `series` capability: detect which series an
    /// edition belongs to. `edition_info` is the table shape
    /// defined in the spec: `{ id, title, isbn, identifiers }`.
    /// Returns the raw JSON value the plugin sent (the typed shape
    /// is in `crate::series::DetectSeriesResult` for callers that
    /// want a typed decode).
    pub async fn call_detect_series(
        &mut self,
        plugin_id: &str,
        edition_info: serde_json::Value,
    ) -> PluginResult<serde_json::Value> {
        let args = vec![edition_info];
        self.call(plugin_id, "detect_series", args).await
    }

    /// Dispatch the `series` capability: get the full ordering for
    /// a detected series. `series_info` is the table shape defined
    /// in the spec: `{ name, external_id, order_type }`. Returns
    /// the raw JSON value the plugin sent (the typed shape is in
    /// `crate::series::SeriesOrderResult` for callers that want a
    /// typed decode).
    pub async fn call_get_series_order(
        &mut self,
        plugin_id: &str,
        series_info: serde_json::Value,
    ) -> PluginResult<serde_json::Value> {
        let args = vec![series_info];
        self.call(plugin_id, "get_series_order", args).await
    }

    /// Dispatch the `search` capability. `query` is the user-typed
    /// search string; `options` is an opaque JSON object the host
    /// passes through to the plugin (per-plugin option keys). Returns
    /// the raw JSON value the plugin sent — typically an array of
    /// hit objects. Plugins are expected to return `[]` (not error)
    /// for an empty query.
    pub async fn call_search(
        &mut self,
        plugin_id: &str,
        query: &str,
        options: serde_json::Value,
    ) -> PluginResult<serde_json::Value> {
        let args = vec![serde_json::Value::String(query.to_string()), options];
        self.call(plugin_id, "search", args).await
    }

    /// Dispatch the `lookup` capability. `identifier` is the
    /// canonical work/edition identifier (ISBN, OCLC, etc.). Returns
    /// the raw JSON value the plugin sent — typically a single hit
    /// object, or `nil` if the plugin has no record for the id.
    pub async fn call_lookup(
        &mut self,
        plugin_id: &str,
        identifier: &str,
    ) -> PluginResult<serde_json::Value> {
        let args = vec![serde_json::Value::String(identifier.to_string())];
        self.call(plugin_id, "lookup", args).await
    }

    /// Dispatch the `enrich` capability. `work_info` is a table
    /// the host already knows about — typically the result of a
    /// prior `search` or `lookup` call. The plugin returns
    /// additional metadata (description, subjects, identifiers)
    /// to fill in gaps in the host's local copy. Returns the raw
    /// JSON value the plugin sent, or `nil` if no enrichment is
    /// available.
    pub async fn call_enrich(
        &mut self,
        plugin_id: &str,
        work_info: serde_json::Value,
    ) -> PluginResult<serde_json::Value> {
        let args = vec![work_info];
        self.call(plugin_id, "enrich", args).await
    }

    /// Dispatch the `cover` capability. `work_info` is the work the
    /// host wants a cover for; `edition_info` is optional context
    /// (specific binding/format) the plugin may use to pick a
    /// better image. Returns the raw JSON value the plugin sent —
    /// typically `{ url, size, source }` or `{ bytes, mime, source }`
    /// depending on the plugin's implementation.
    pub async fn call_get_cover(
        &mut self,
        plugin_id: &str,
        work_info: serde_json::Value,
        edition_info: Option<serde_json::Value>,
    ) -> PluginResult<serde_json::Value> {
        let args = vec![work_info, edition_info.unwrap_or(serde_json::Value::Null)];
        self.call(plugin_id, "get_cover", args).await
    }

    /// Dispatch the `watch` capability. `since` is an opaque
    /// cursor the plugin returned from a prior call (or `None`
    /// for the first poll). The plugin returns a batch of
    /// change items it knows about since that point, plus an
    /// optional `next_cursor` for the host to feed back in on
    /// the next poll and a `has_more` flag indicating whether
    /// more results are available.
    ///
    /// This is a polling capability: one call, one batch. The
    /// dispatcher does not maintain a long-running thread, and
    /// the Tauri event bus is intentionally not involved here
    /// (a future work item may add push-style notifications on
    /// top of this polling primitive — see
    /// `docs/plans/2026-06-07-plugin-roadmap.md` P3 §"Watch
    /// capability").
    pub async fn call_watch(
        &mut self,
        plugin_id: &str,
        since: Option<String>,
    ) -> PluginResult<crate::watch::WatchResult> {
        let args = match since {
            Some(s) => vec![serde_json::Value::String(s)],
            None => Vec::new(),
        };
        let raw = self.call(plugin_id, "watch", args).await?;
        serde_json::from_value(raw)
            .map_err(|e| PluginError::Serialization(format!("invalid watch response: {e}")))
    }

    /// Dispatch the `import_detect` capability. `source` is a JSON
    /// value the UI packs (file path, URL, directory). Returns the
    /// raw JSON value the plugin sent — a table with `confidence`,
    /// `format_name?`, `estimated_count?` — or `nil` if the plugin
    /// declines the source.
    pub async fn call_import_detect(
        &mut self,
        plugin_id: &str,
        source: serde_json::Value,
    ) -> PluginResult<serde_json::Value> {
        let args = vec![source];
        self.call(plugin_id, "import_detect", args).await
    }

    /// Dispatch the `import_list_items` capability. Returns the raw
    /// JSON value the plugin sent — an array of preview records.
    pub async fn call_import_list_items(
        &mut self,
        plugin_id: &str,
        source: serde_json::Value,
        options: serde_json::Value,
    ) -> PluginResult<serde_json::Value> {
        let args = vec![source, options];
        self.call(plugin_id, "import_list_items", args).await
    }

    /// Dispatch the `import_items` capability. `options` carries
    /// `selected_items: Vec<String>` (the ids from the preview list)
    /// and the plugin's runtime config. Returns the raw JSON value
    /// the plugin sent — an array of `ImportRecord` values.
    pub async fn call_import_items(
        &mut self,
        plugin_id: &str,
        source: serde_json::Value,
        options: serde_json::Value,
    ) -> PluginResult<serde_json::Value> {
        let args = vec![source, options];
        self.call(plugin_id, "import_items", args).await
    }

    pub async fn shutdown(&mut self) -> PluginResult<()> {
        self.send_main(&MainToHost::Shutdown).await?;
        self.stdin.lock().await.shutdown().await.ok();

        // Wait for the child to exit with a generous timeout.
        // On macOS, `ChildStdin::shutdown()` is a socket-only
        // operation that silently fails on pipes, so the child
        // may not receive EOF.  If the child hasn't exited
        // within 5 seconds, kill it to guarantee shutdown
        // completes in bounded time regardless of platform.
        match tokio::time::timeout(Duration::from_secs(5), self.child.wait()).await {
            Ok(status) => {
                tracing::debug!("plugin host exited with {status:?}");
            }
            Err(_) => {
                tracing::warn!("plugin host did not exit in time, killing");
                self.child.kill().await.ok();
                self.child.wait().await.ok();
            }
        }

        self.pending_calls.clear();
        Ok(())
    }

    pub fn list_plugins(&self) -> Vec<&PluginManifest> {
        self.loaded_plugins
            .iter()
            .filter(|(id, _)| !self.disabled.contains(*id))
            .flat_map(|(_, p)| p.versions.values())
            .map(|pv| &pv.manifest)
            .collect()
    }

    /// Add or remove a plugin id from the in-memory disabled set.
    /// The Tauri command layer mirrors the same change to
    /// `installed.json` on disk so the choice survives restarts.
    pub fn set_disabled(&mut self, id: &str, disabled: bool) {
        if disabled {
            self.disabled.insert(id.to_string());
        } else {
            self.disabled.remove(id);
        }
    }

    /// True if the user has disabled this plugin. Disabled plugins
    /// are filtered from `list_plugins()` and from any "enabled
    /// plugins" view in the UI.
    pub fn is_disabled(&self, id: &str) -> bool {
        self.disabled.contains(id)
    }

    pub fn list_loaded_ids(&self) -> Vec<(&str, &str)> {
        self.loaded_plugins
            .iter()
            .filter_map(|(id, p)| {
                let active = p.active_version.as_deref()?;
                let pv = p.versions.get(active)?;
                if pv.load_state == LOAD_STATE_LOADED {
                    Some((id.as_str(), pv.manifest.plugin.name.as_str()))
                } else {
                    None
                }
            })
            .collect()
    }

    /// True when the plugin's currently-active version has finished
    /// loading on the sidecar (i.e. its `LoadPlugin` IPC completed
    /// without error). `false` when the plugin is only discovered,
    /// disabled, or its active version failed to load. The frontend's
    /// `PluginInfo.loaded` mirrors this so the UI never offers a
    /// capability that will fail at dispatch time.
    pub fn is_active_version_loaded(&self, id: &str) -> bool {
        let Some(entry) = self.loaded_plugins.get(id) else {
            return false;
        };
        let Some(active) = entry.active_version.as_deref() else {
            return false;
        };
        let Some(pv) = entry.versions.get(active) else {
            return false;
        };
        pv.load_state == LOAD_STATE_LOADED
    }

    pub fn runtime(&self) -> &str {
        &self.runtime
    }

    async fn send_main(&self, msg: &MainToHost) -> PluginResult<()> {
        let payload = rmp_serde::to_vec_named(msg).map_err(|e| PluginError::Ipc(e.to_string()))?;
        let len = (payload.len() as u32).to_le_bytes();
        let mut guard = self.stdin.lock().await;
        guard.write_all(&len).await?;
        guard.write_all(&payload).await?;
        guard.flush().await?;
        Ok(())
    }

    /// Like [`Self::send_main`] but for `MainToHostCallback`
    /// variants (the reply channel the host process uses for
    /// its oneshot router). Used by [`Self::dispatch`] to
    /// answer request variants that the host function is
    /// blocked on. The wire format is identical to `send_main`
    /// — length-prefixed MessagePack over stdin — only the
    /// `serde` type differs.
    async fn send_callback(&self, msg: &MainToHostCallback) -> PluginResult<()> {
        let payload = rmp_serde::to_vec_named(msg).map_err(|e| PluginError::Ipc(e.to_string()))?;
        let len = (payload.len() as u32).to_le_bytes();
        let mut guard = self.stdin.lock().await;
        guard.write_all(&len).await?;
        guard.write_all(&payload).await?;
        guard.flush().await?;
        Ok(())
    }

    async fn recv(&mut self) -> PluginResult<HostToMain> {
        use tokio::io::AsyncReadExt;
        let mut len_buf = [0u8; 4];
        self.stdout
            .read_exact(&mut len_buf)
            .await
            .map_err(|e| PluginError::Ipc(format!("read len: {e}")))?;
        let len = u32::from_le_bytes(len_buf) as usize;
        let mut payload = vec![0u8; len];
        self.stdout
            .read_exact(&mut payload)
            .await
            .map_err(|e| PluginError::Ipc(format!("read payload: {e}")))?;
        rmp_serde::from_slice(&payload).map_err(|e| {
            let preview = String::from_utf8_lossy(&payload[..payload.len().min(512)]);
            PluginError::Ipc(format!(
                "{e} (payload_len={}, preview={preview:?})",
                payload.len()
            ))
        })
    }

    /// Process a single `HostToMain` request message and send the
    /// matching `MainToHostCallback` back to the host over the
    /// IPC channel.
    ///
    /// Returns `true` when the message was a recognised request
    /// that needs a response (caller can stop watching for it),
    /// `false` when the message is fire-and-forget (`Log`,
    /// `HttpRequest`, `EmitEvent`) or unhandled.
    ///
    /// This is the dispatcher that wires the host-side
    /// `host.resolve_identifier` / `host.get_edition_info`
    /// functions (registered in `host_lua.rs`) to the main-side
    /// `livtet-core` queries. Without it, the host function would
    /// block on its oneshot for the full REQUEST_TIMEOUT before
    /// the host process gave up and propagated an error to the
    /// Lua caller.
    /// Returns the data directory (parent of plugin_dir) for storing
    /// fallback secret files when the OS keyring is unavailable.
    fn data_dir(&self) -> &Utf8Path {
        self.plugin_dir
            .parent()
            .expect("plugin_dir should have a parent directory")
    }

    /// Read a secret from the OS keyring, falling back to an encrypted
    /// file if the keyring is unavailable (headless environments).
    async fn handle_secret_read(
        &self,
        request_id: &str,
        plugin_id: &str,
        name: &str,
    ) -> MainToHostCallback {
        let key = format!("{plugin_id}:{name}");
        match keyring::Entry::new(livtet_core::paths::BUNDLE_ID, &key) {
            Ok(entry) => match entry.get_password() {
                Ok(value) => MainToHostCallback::SecretResult {
                    id: request_id.to_string(),
                    value: Some(value),
                    error: None,
                },
                // The `(value = None, error = None)` shape is
                // reserved for "secret exists but is empty"; the
                // caller used to also see this for missing keys
                // and couldn't tell the difference. Now we
                // surface a clear `"missing"` error string so
                // plugins can branch.
                Err(keyring::Error::NoEntry) => MainToHostCallback::SecretResult {
                    id: request_id.to_string(),
                    value: None,
                    error: Some(format!("secret not found: {plugin_id}:{name}")),
                },
                Err(e) => MainToHostCallback::SecretResult {
                    id: request_id.to_string(),
                    value: None,
                    error: Some(format!("keyring read error: {e}")),
                },
            },
            Err(_) => {
                // Keyring unavailable, fall back to encrypted file
                self.read_secret_fallback(request_id, plugin_id, name).await
            }
        }
    }

    /// Write a secret to the OS keyring, falling back to an encrypted
    /// file if the keyring is unavailable (headless environments).
    async fn handle_secret_write(
        &self,
        request_id: &str,
        plugin_id: &str,
        name: &str,
        value: &str,
    ) -> MainToHostCallback {
        let key = format!("{plugin_id}:{name}");
        match keyring::Entry::new(livtet_core::paths::BUNDLE_ID, &key) {
            Ok(entry) => match entry.set_password(value) {
                Ok(()) => MainToHostCallback::SecretResult {
                    id: request_id.to_string(),
                    value: None,
                    error: None,
                },
                Err(e) => MainToHostCallback::SecretResult {
                    id: request_id.to_string(),
                    value: None,
                    error: Some(format!("keyring write error: {e}")),
                },
            },
            Err(_) => {
                // Keyring unavailable, fall back to encrypted file
                self.write_secret_fallback(request_id, plugin_id, name, value)
                    .await
            }
        }
    }

    /// Read every persisted setting for the given plugin from the
    /// `plugin_settings` table. Returns `None` if no DB pool is
    /// available; returns `Some(empty_map)` if the DB has no rows
    /// for this plugin (a successful "no settings yet" answer
    /// is distinct from "we couldn't ask"). The host process
    /// treats `None` the same as an empty map today; the
    /// distinction matters when callers want to log a one-shot
    /// warning.
    /// Read a single `(plugin_id, key)` value from the `plugin_settings`
    /// DB. Returns `None` if the row doesn't exist. Mirror of the
    /// `plugin_get_setting` Tauri command.
    pub async fn get_setting(&self, plugin_id: &str, key: &str) -> Option<String> {
        let pool = self.db.as_ref()?;
        let row: Option<(String,)> = sql::query_as(AssertSqlSafe(
            "SELECT value_json FROM plugin_settings WHERE plugin_id = ? AND setting_key = ?",
        ))
        .bind(plugin_id)
        .bind(key)
        .fetch_optional(pool)
        .await
        .ok()
        .flatten();
        row.map(|(v,)| v)
    }

    async fn load_plugin_settings(
        &self,
        plugin_id: &str,
    ) -> Option<std::collections::HashMap<String, String>> {
        use sql::Row;
        let pool = self.db.as_ref()?;
        let rows = match sql::query(AssertSqlSafe(
            "SELECT setting_key, value_json FROM plugin_settings WHERE plugin_id = ?",
        ))
        .bind(plugin_id)
        .fetch_all(pool)
        .await
        {
            Ok(rs) => rs,
            Err(e) => {
                warn!(plugin = %plugin_id, error = %e, "plugin_settings read failed");
                return None;
            }
        };
        let mut out = std::collections::HashMap::with_capacity(rows.len());
        for row in rows {
            let key: String = row.try_get("setting_key").ok()?;
            let value: String = row.try_get("value_json").ok()?;
            out.insert(key, value);
        }
        Some(out)
    }

    /// Persist a plugin setting to the `plugin_settings` DB
    /// table. Returns a [`MainToHostCallback::SettingResult`] with
    /// `error: Some(_)` if no DB pool is available or the write
    /// fails. Upsert: try `UPDATE` first; fall back to `INSERT`
    /// when no row exists for this `(plugin_id, key)` pair.
    async fn handle_setting_write(
        &self,
        request_id: &str,
        plugin_id: &str,
        key: &str,
        value: &str,
    ) -> MainToHostCallback {
        match self.write_setting_direct(plugin_id, key, value).await {
            Ok(()) => MainToHostCallback::SettingResult {
                id: request_id.to_string(),
                ok: true,
                error: None,
            },
            Err(e) => MainToHostCallback::SettingResult {
                id: request_id.to_string(),
                ok: false,
                error: Some(e.to_string()),
            },
        }
    }

    /// Tauri-command-side helper for `plugin_save_setting`. Writes
    /// directly to the `plugin_settings` DB without a sidecar
    /// round-trip. Also reused by the IPC dispatcher arm above.
    /// Returns `Err` if no DB pool is available or the write
    /// fails.
    pub async fn write_setting_direct(
        &self,
        plugin_id: &str,
        key: &str,
        value: &str,
    ) -> PluginResult<()> {
        let Some(pool) = self.db.as_ref() else {
            return Err(PluginError::Ipc(
                "settings require a database pool".to_string(),
            ));
        };
        let updated = sql::query(AssertSqlSafe(
            "UPDATE plugin_settings SET value_json = ?, updated_at = datetime('now') \
             WHERE plugin_id = ? AND setting_key = ?",
        ))
        .bind(value)
        .bind(plugin_id)
        .bind(key)
        .execute(pool)
        .await
        .map_err(|e| PluginError::Ipc(format!("plugin_settings update: {e}")))?;
        if updated.rows_affected() > 0 {
            return Ok(());
        }
        // No existing row; insert a new one.
        let id = livtet_core::DbId::new();
        sql::query(AssertSqlSafe(
            "INSERT INTO plugin_settings (id, plugin_id, setting_key, value_json, created_at, updated_at) \
             VALUES (?, ?, ?, ?, datetime('now'), datetime('now'))",
))
        .bind(id.to_bytes().to_vec())
        .bind(plugin_id)
        .bind(key)
        .bind(value)
        .execute(pool)
        .await
        .map_err(|e| PluginError::Ipc(format!("plugin_settings insert: {e}")))?;
        Ok(())
    }

    /// Read a secret from the encrypted fallback file.
    async fn read_secret_fallback(
        &self,
        request_id: &str,
        plugin_id: &str,
        name: &str,
    ) -> MainToHostCallback {
        let secrets_file = self.data_dir().join("plugin-secrets.json");

        // Load or derive the encryption key
        let hmac_key = self.hmac_key.clone();

        // Read and decrypt the secrets file
        let secrets: HashMap<String, String> = match fs::read_to_string(&secrets_file) {
            Ok(content) => {
                let decoded = match base64::Engine::decode(
                    &base64::engine::general_purpose::STANDARD,
                    content.trim(),
                ) {
                    Ok(d) => d,
                    Err(e) => {
                        return MainToHostCallback::SecretResult {
                            id: request_id.to_string(),
                            value: None,
                            error: Some(format!("base64 decode error: {e}")),
                        };
                    }
                };

                if decoded.len() < 12 {
                    return MainToHostCallback::SecretResult {
                        id: request_id.to_string(),
                        value: None,
                        error: Some("encrypted data too short".to_string()),
                    };
                }

                let cipher = match Aes256Gcm::new_from_slice(hmac_key.as_bytes()) {
                    Ok(c) => c,
                    Err(e) => {
                        return MainToHostCallback::SecretResult {
                            id: request_id.to_string(),
                            value: None,
                            error: Some(format!("cipher init error: {e}")),
                        };
                    }
                };

                let nonce = match Nonce::try_from(&decoded[..12]) {
                    Ok(n) => n,
                    Err(_) => {
                        return MainToHostCallback::SecretResult {
                            id: request_id.to_string(),
                            value: None,
                            error: Some("nonce conversion failed".to_string()),
                        };
                    }
                };
                let ciphertext = &decoded[12..];

                match cipher.decrypt(&nonce, ciphertext) {
                    Ok(plaintext) => match String::from_utf8(plaintext) {
                        Ok(json) => serde_json::from_str(&json).unwrap_or_default(),
                        Err(_) => HashMap::new(),
                    },
                    Err(_) => HashMap::new(),
                }
            }
            Err(_) => HashMap::new(),
        };

        let full_key = format!("{}:{}", plugin_id, name);
        match secrets.get(&full_key) {
            Some(value) => MainToHostCallback::SecretResult {
                id: request_id.to_string(),
                value: Some(value.clone()),
                error: None,
            },
            // Same fix as `handle_secret_read`: surface a
            // dedicated `"missing"` error so callers can tell
            // "no such secret" from "secret exists and is empty".
            None => MainToHostCallback::SecretResult {
                id: request_id.to_string(),
                value: None,
                error: Some(format!("secret not found: {plugin_id}:{name}")),
            },
        }
    }

    /// Write a secret to the encrypted fallback file.
    async fn write_secret_fallback(
        &self,
        request_id: &str,
        plugin_id: &str,
        name: &str,
        value: &str,
    ) -> MainToHostCallback {
        let secrets_file = self.data_dir().join("plugin-secrets.json");

        // Load or derive the encryption key
        let hmac_key = self.hmac_key.clone();

        let mut secrets: HashMap<String, String> = match fs::read_to_string(&secrets_file) {
            Ok(content) => {
                decrypt_secrets_content(&content, hmac_key.as_bytes()).unwrap_or_default()
            }
            Err(_) => HashMap::new(),
        };

        // Update the secret
        let full_key = format!("{}:{}", plugin_id, name);
        secrets.insert(full_key, value.to_string());

        // Serialize and encrypt
        let json = match serde_json::to_string(&secrets) {
            Ok(j) => j,
            Err(e) => {
                return MainToHostCallback::SecretResult {
                    id: request_id.to_string(),
                    value: None,
                    error: Some(format!("json serialize error: {e}")),
                };
            }
        };

        let cipher = match Aes256Gcm::new_from_slice(hmac_key.as_bytes()) {
            Ok(c) => c,
            Err(e) => {
                return MainToHostCallback::SecretResult {
                    id: request_id.to_string(),
                    value: None,
                    error: Some(format!("cipher init error: {e}")),
                };
            }
        };

        // Generate random nonce
        let mut nonce_bytes = [0u8; 12];
        rng().fill_bytes(&mut nonce_bytes);
        let nonce = Nonce::try_from(&nonce_bytes[..]).unwrap(); // Always succeeds: exactly 12 bytes

        let ciphertext = match cipher.encrypt(&nonce, json.as_bytes()) {
            Ok(c) => c,
            Err(e) => {
                return MainToHostCallback::SecretResult {
                    id: request_id.to_string(),
                    value: None,
                    error: Some(format!("encryption error: {e}")),
                };
            }
        };

        // Prepend nonce to ciphertext and base64 encode
        let mut combined = nonce_bytes.to_vec();
        combined.extend(ciphertext);
        let encoded = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &combined);

        // Write to file
        if let Err(e) = fs::write(&secrets_file, encoded) {
            return MainToHostCallback::SecretResult {
                id: request_id.to_string(),
                value: None,
                error: Some(format!("write secrets file error: {e}")),
            };
        }

        MainToHostCallback::SecretResult {
            id: request_id.to_string(),
            value: None,
            error: None,
        }
    }

    pub async fn dispatch(&self, msg: HostToMain) -> PluginResult<bool> {
        match msg {
            // TBD: `get_edition_info` was the last DB-backed host
            // bridge left after the CRUD module was deleted. Restore
            // once a replacement bridge ships.
            HostToMain::GetEditionInfoRequest { id, edition_id, .. } => {
                let response = self.handle_get_edition_info(&id, &edition_id).await;
                self.send_callback(&response).await?;
                Ok(true)
            }
            HostToMain::ResolveIdentifierRequest { .. }
            | HostToMain::ResolveIdentifiersRequest { .. }
            | HostToMain::GetEditionIdentifiersRequest { .. }
            | HostToMain::FetchProgressRequest { .. }
            | HostToMain::UpsertProgressRequest { .. } => {
                // DB-backed identifier/progress bridges were removed when
                // `livtet_core::crud` and the identifier-resolution helpers
                // were deleted. The host refuses these requests until a
                // replacement is wired up.
                warn!("received dropped DB-bridge request; ignoring");
                Ok(false)
            }
            HostToMain::Log { .. }
            | HostToMain::HttpRequest { .. }
            | HostToMain::EmitEvent { .. } => Ok(false),
            HostToMain::SecretRequest {
                id,
                plugin_id,
                name,
            } => {
                let response = self.handle_secret_read(&id, &plugin_id, &name).await;
                self.send_callback(&response).await?;
                Ok(true)
            }
            HostToMain::SetSecretRequest {
                id,
                plugin_id,
                name,
                value,
            } => {
                let response = self
                    .handle_secret_write(&id, &plugin_id, &name, &value)
                    .await;
                self.send_callback(&response).await?;
                Ok(true)
            }
            HostToMain::SetSettingRequest {
                id,
                plugin_id,
                key,
                value,
            } => {
                let response = self
                    .handle_setting_write(&id, &plugin_id, &key, &value)
                    .await;
                self.send_callback(&response).await?;
                Ok(true)
            }
            HostToMain::OAuthRedeemRequest {
                id,
                plugin_id,
                provider,
            } => {
                if let Some(ref handler) = self.oauth_handler {
                    let response = handler.handle_redeem_token(id, plugin_id, provider).await;
                    self.send_callback(&response).await?;
                }
                Ok(true)
            }
            HostToMain::OAuthTokenLookupRequest {
                id,
                plugin_id,
                provider,
            } => {
                if let Some(ref handler) = self.oauth_handler {
                    let response = handler
                        .handle_get_valid_token(id, plugin_id, provider)
                        .await;
                    self.send_callback(&response).await?;
                }
                Ok(true)
            }
            HostToMain::OAuthRevokeRequest {
                id,
                plugin_id,
                provider,
            } => {
                if let Some(ref handler) = self.oauth_handler {
                    let response = handler.handle_revoke_token(id, plugin_id, provider).await;
                    self.send_callback(&response).await?;
                }
                Ok(true)
            }
            HostToMain::OAuthAuthorizeRequest {
                id,
                plugin_id,
                provider,
            } => {
                if let Some(ref handler) = self.oauth_handler {
                    let response = handler.handle_authorize(id, plugin_id, provider).await;
                    self.send_callback(&response).await?;
                }
                Ok(true)
            }
            other => {
                // Unknown request variant. Reply with an error so
                // the host function fails fast instead of hanging
                // on its oneshot for the full timeout. The reply
                // piggybacks on `ResolveIdentifierResult` because
                // that's the one MainToHostCallback variant the
                // host process always knows how to route by id;
                // the host-side router ignores the `edition_id`
                // field and surfaces the `error` string to the
                // caller.
                let id = callback_request_id(&other);
                if let Some(req_id) = id {
                    self.send_callback(&MainToHostCallback::ResolveIdentifierResult {
                        id: req_id.to_string(),
                        edition_id: None,
                        error: Some(format!("unhandled request: {other:?}")),
                    })
                    .await?;
                    Ok(true)
                } else {
                    warn!(message = ?other, "unhandled host message");
                    Ok(false)
                }
            }
        }
    }

    /// TBD: `handle_get_edition_info` was the last DB-backed host
    /// bridge left after the CRUD module was deleted. Restore once a
    /// replacement bridge ships.
    async fn handle_get_edition_info(
        &self,
        _request_id: &str,
        _edition_id: &str,
    ) -> MainToHostCallback {
        panic!(
            "TBD: handle_get_edition_info host bridge removed; see livtet-plugins/src/host_manager.rs"
        )
    }
}

impl crate::host_trait::HostBase for PluginHostManager {}

impl crate::host_trait::HostSystemSecrets for PluginHostManager {
    fn get_system_secret(&self, name: PluginSystemSecret) -> Option<String> {
        self.system_secrets.get(&name).cloned()
    }
}

fn forward_log(plugin_id: &str, level: &str, message: &str) {
    match level.to_ascii_lowercase().as_str() {
        "error" => error!(plugin = %plugin_id, "{}", message),
        "warn" => warn!(plugin = %plugin_id, "{}", message),
        _ => info!(plugin = %plugin_id, "{}", message),
    }
}

fn callback_request_id(msg: &HostToMain) -> Option<&str> {
    match msg {
        HostToMain::ResolveIdentifierRequest { id, .. }
        | HostToMain::ResolveIdentifiersRequest { id, .. }
        | HostToMain::GetEditionInfoRequest { id, .. }
        | HostToMain::GetEditionIdentifiersRequest { id, .. }
        | HostToMain::FetchProgressRequest { id, .. }
        | HostToMain::UpsertProgressRequest { id, .. }
        | HostToMain::SecretRequest { id, .. }
        | HostToMain::SetSecretRequest { id, .. }
        | HostToMain::SetSettingRequest { id, .. }
        | HostToMain::ReadFileRequest { id, .. }
        | HostToMain::SqliteQueryRequest { id, .. }
        | HostToMain::ReadAssetRequest { id, .. } => Some(id.as_str()),
        _ => None,
    }
}

#[cfg(test)]
mod build_command_tests {
    //! Regression tests for the sidecar argv construction.
    //!
    //! The host's clap CLI only declares the optional `--log-dir`
    //! long flag. Passing any positional argument causes the host
    //! to exit 2 before writing the IPC `Ready` handshake, which
    //! the parent surfaces as `ipc error: read len: early eof`.
    //!
    //! These tests spawn a tiny recording shell script and assert
    //! that the actual `tokio::process::Command` produces the
    //! expected argv and env at process launch — `tokio`'s
    //! `Command` does not expose `get_args` / `get_envs`, so this
    //! is the only way to test the sidecar's argv contract.

    use super::*;

    /// Spawn a real child via `build_command` and assert the
    /// observed argv contains only the binary path. Uses a tiny
    /// shell script that prints `$@` to its own stdout; the test
    /// captures and inspects it. Exits 0 regardless of argv.
    #[tokio::test]
    async fn build_command_spawned_child_sees_no_positional_args() {
        let tmp = camino_tempfile::tempdir().expect("tempdir");
        let script = tmp.path().join("print-args.sh");
        let script_contents = r#"#!/bin/sh
printf 'ARGC=%d\n' "$#"
exit 0
"#;
        std::fs::write(&script, script_contents).expect("write script");
        std::fs::set_permissions(&script, std::os::unix::fs::PermissionsExt::from_mode(0o755))
            .expect("chmod script");

        let binary = script.as_path();
        let mut command = build_command(binary, None, &CommandEnv::default());
        command.stdin(std::process::Stdio::null());
        command.stdout(std::process::Stdio::piped());

        let output = command.output().await.expect("spawn script");
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            stdout.contains("ARGC=0"),
            "child should observe zero positional args; got {stdout:?}"
        );
    }

    /// Spawn a child with `log_dir = Some(...)` and assert the
    /// `LIVTET_LOG_DIR` env var is set to the supplied path and the
    /// argv is still empty.
    #[tokio::test]
    async fn build_command_spawned_child_sets_livtet_log_dir_when_log_dir_is_some() {
        let tmp = camino_tempfile::tempdir().expect("tempdir");
        let script = tmp.path().join("print-env.sh");
        let script_contents = r#"#!/bin/sh
printf 'ARGC=%d\n' "$#"
printf 'LIVTET_LOG_DIR=%s\n' "${LIVTET_LOG_DIR-}"
exit 0
"#;
        std::fs::write(&script, script_contents).expect("write script");
        std::fs::set_permissions(&script, std::os::unix::fs::PermissionsExt::from_mode(0o755))
            .expect("chmod script");

        let binary = script.as_path();
        let log_dir = Utf8Path::new("/tmp/livtet-logs");
        let mut command = build_command(binary, Some(log_dir), &CommandEnv::default());
        command.stdin(std::process::Stdio::null());
        command.stdout(std::process::Stdio::piped());

        let output = command.output().await.expect("spawn script");
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            stdout.contains("ARGC=0"),
            "child should observe zero positional args; got {stdout:?}"
        );
        assert!(
            stdout.contains("LIVTET_LOG_DIR=/tmp/livtet-logs"),
            "child should observe LIVTET_LOG_DIR=/tmp/livtet-logs; got {stdout:?}"
        );
    }

    /// Spawn a child with `log_dir = None` and assert the
    /// `LIVTET_LOG_DIR` env var is unset.
    #[tokio::test]
    async fn build_command_spawned_child_omits_livtet_log_dir_when_log_dir_is_none() {
        let tmp = camino_tempfile::tempdir().expect("tempdir");
        let script = tmp.path().join("print-env.sh");
        let script_contents = r#"#!/bin/sh
printf 'LIVTET_LOG_DIR=%s\n' "${LIVTET_LOG_DIR-UNSET}"
exit 0
"#;
        std::fs::write(&script, script_contents).expect("write script");
        std::fs::set_permissions(&script, std::os::unix::fs::PermissionsExt::from_mode(0o755))
            .expect("chmod script");

        let binary = script.as_path();
        let mut command = build_command(binary, None, &CommandEnv::default());
        command.stdin(std::process::Stdio::null());
        command.stdout(std::process::Stdio::piped());

        let output = command.output().await.expect("spawn script");
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            stdout.contains("LIVTET_LOG_DIR=UNSET"),
            "LIVTET_LOG_DIR must be unset when log_dir is None; got {stdout:?}"
        );
    }

    /// Spawn a child with `command_env` setting `LUA_PATH` and
    /// `LUA_CPATH` and assert the env vars reach the child. Pins
    /// the `LUA_PATH` / `LUA_CPATH` env propagation contract the
    /// Tauri parent relies on when wiring the sidecar to a
    /// `luarocks::build_env(app_data_dir)` result.
    #[tokio::test]
    async fn build_command_spawned_child_sets_lua_path_and_lua_cpath() {
        let tmp = camino_tempfile::tempdir().expect("tempdir");
        let script = tmp.path().join("print-env.sh");
        let script_contents = r#"#!/bin/sh
printf 'LUA_PATH=%s\n' "${LUA_PATH-UNSET}"
printf 'LUA_CPATH=%s\n' "${LUA_CPATH-UNSET}"
exit 0
"#;
        std::fs::write(&script, script_contents).expect("write script");
        std::fs::set_permissions(&script, std::os::unix::fs::PermissionsExt::from_mode(0o755))
            .expect("chmod script");

        let binary = script.as_path();
        let env = CommandEnv {
            lua_path: Some(
                "/tmp/foo/share/lua/5.4/?.lua;/tmp/foo/share/lua/5.4/?/init.lua;;".to_string(),
            ),
            lua_cpath: Some(
                "/tmp/foo/lib/lua/5.4/?.so;/tmp/foo/lib/lua/5.4/loadall.so;;".to_string(),
            ),
        };
        let mut command = build_command(binary, None, &env);
        command.stdin(std::process::Stdio::null());
        command.stdout(std::process::Stdio::piped());

        let output = command.output().await.expect("spawn script");
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            stdout.contains("LUA_PATH=/tmp/foo/share/lua/5.4/?.lua"),
            "LUA_PATH should be set; got {stdout:?}"
        );
        assert!(
            stdout.contains("LUA_CPATH=/tmp/foo/lib/lua/5.4/?.so"),
            "LUA_CPATH should be set; got {stdout:?}"
        );
    }
}
