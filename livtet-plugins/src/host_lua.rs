use std::{
    collections::HashMap,
    str::FromStr,
    sync::{Arc, Mutex},
};

use hex;
use mlua::{
    Function, HookTriggers, Lua, LuaOptions, LuaSerdeExt, StdLib, Table, UserData, UserDataMethods,
    Value, VmState,
};
use scraper::{Html as ScraperHtml, Selector};
use livtet_data::sql::AssertSqlSafe;

use crate::{
    error::PluginError,
    host_trait::{
        HostBase, HostDatabase, HostEmbeddings, HostFiles, HostHttp, HostLog, HostOAuth,
        HostSecrets, HostSettings, HostSystemSecrets, SandboxConfig,
    },
    permissions::{
        ResolvedGrant, check_embeddings, check_oauth, check_read, check_sqlite,
        check_system_secret, check_write, load_grant, missing_sidecar_error, outside_glob_error,
        permissions_dir, system_secret_denied_error,
    },
    protocol::{HostToMain, MainToHost},
    system_secrets::PluginSystemSecret,
};

macro_rules! load_grant {
    ($grants:expr, $perms_dir:expr, $lua:expr) => {{
        let plugin_id = read_current_plugin_id($lua);
        match load_or_cached_grant($grants, &plugin_id, $perms_dir) {
            Ok(Some(g)) => g,
            Ok(None) => return Ok((None, Some(missing_sidecar_error(&plugin_id)))),
            Err(e) => return Ok((None, Some(format!("permission error: {e}")))),
        }
    }};
}

struct PluginEntry {
    provider: Table,
    data_dir: Option<camino::Utf8PathBuf>,
}

type LoadedIds = Arc<Mutex<std::collections::HashSet<String>>>;
type PluginSettings = Arc<Mutex<HashMap<String, HashMap<String, String>>>>;
type PluginGrants = Arc<Mutex<HashMap<String, Option<Arc<ResolvedGrant>>>>>;

const CURRENT_PLUGIN_ID_GLOBAL: &str = "__livtet_current_plugin_id";
const CURRENT_PLUGIN_DIR_GLOBAL: &str = "__livtet_current_plugin_dir";
const CURRENT_CALL_ID_GLOBAL: &str = "__livtet_current_call_id";

pub struct LuaHost<H: HostBase + HostHttp + HostLog + HostSystemSecrets + HostOAuth + 'static> {
    lua: Lua,
    loaded_plugins: HashMap<String, PluginEntry>,
    loaded_ids: LoadedIds,
    plugin_settings: PluginSettings,
    plugin_grants: PluginGrants,
    /// Tracks which plugins have declared `system_secrets = true`
    /// in their `[plugin.requires]`. Populated via
    /// `declare_system_secrets` at load time; consumed by the
    /// system-secret bridge for the manifest-side half of the
    /// two-gate check.
    system_secrets_declared: Arc<Mutex<HashMap<String, bool>>>,
    host_impl: Arc<H>,
    /// Cached return values of `host.require(name)` calls. The sandbox
    /// strips `package` so we can't rely on Lua's `package.loaded` for
    /// caching; we maintain our own map keyed by rock name. Without this,
    /// every `host.require("dkjson")` re-executes the 713-line chunk.
    #[allow(clippy::arc_with_non_send_sync)]
    require_cache: Arc<Mutex<HashMap<String, Value>>>,
}

impl<H> LuaHost<H>
where
    H: HostBase
        + HostHttp
        + HostLog
        + HostSecrets
        + HostSystemSecrets
        + HostSettings
        + HostDatabase
        + HostFiles
        + HostEmbeddings
        + HostOAuth
        + 'static,
{
    pub fn new(host_impl: Arc<H>) -> mlua::Result<Self> {
        Self::with_config(host_impl, &DefaultSandbox)
    }

    #[allow(clippy::arc_with_non_send_sync)]
    pub fn with_config(host_impl: Arc<H>, config: &dyn SandboxConfig) -> mlua::Result<Self> {
        let lua = Self::build_sandboxed_lua(config)?;
        let loaded_ids: LoadedIds = Arc::new(Mutex::new(std::collections::HashSet::new()));
        let plugin_settings: PluginSettings = Arc::new(Mutex::new(HashMap::new()));
        let plugin_grants: PluginGrants = Arc::new(Mutex::new(HashMap::new()));
        let mut host = Self {
            lua,
            loaded_plugins: HashMap::new(),
            loaded_ids,
            plugin_settings,
            plugin_grants,
            system_secrets_declared: Arc::new(Mutex::new(HashMap::new())),
            host_impl,
            require_cache: Arc::new(Mutex::new(HashMap::new())),
        };
        host.setup_host_functions()?;
        Ok(host)
    }

    /// Record whether a plugin declares the `system_secrets`
    /// require in its `[plugin.requires]` table. The host
    /// manager (or test harness) calls this at plugin load time
    /// so the system-secret bridge can cross-check the manifest
    /// declaration against the grant sidecar allowlist.
    pub fn declare_system_secrets(&self, plugin_id: &str, declared: bool) {
        let mut cache = self
            .system_secrets_declared
            .lock()
            .map_err(|e| format!("system_secrets_declared lock poisoned: {e}"))
            .expect("lock");
        cache.insert(plugin_id.to_string(), declared);
    }

    /// Pre-populate the grant cache for a plugin. Production code
    /// reads grants from sidecar files at `~/.local/share/livtet/
    /// permissions/<plugin>.{toml,json}`, but the test harness needs
    /// a way to skip the filesystem and inject a grant in-process so
    /// plugin loading stays hermetic.
    pub fn grant_plugin(
        &self,
        plugin_id: &str,
        grant: std::sync::Arc<ResolvedGrant>,
    ) -> Result<(), String> {
        let mut cache = self
            .plugin_grants
            .lock()
            .map_err(|e| format!("plugin_grants lock poisoned: {e}"))?;
        cache.insert(plugin_id.to_string(), Some(grant));
        Ok(())
    }

    fn build_sandboxed_lua(config: &dyn SandboxConfig) -> mlua::Result<Lua> {
        let libs = StdLib::COROUTINE | StdLib::TABLE | StdLib::STRING | StdLib::UTF8 | StdLib::MATH;
        let lua = Lua::new_with(libs, LuaOptions::default())?;
        lua.set_memory_limit(config.memory_limit())?;
        let globals = lua.globals();
        let _ = globals.set("os", Value::Nil);
        let _ = globals.set("io", Value::Nil);
        let _ = globals.set("debug", Value::Nil);
        let package: Value = globals.get("package").unwrap_or(Value::Nil);
        if !matches!(package, Value::Nil) {
            let tbl: Table = globals.get("package")?;
            let _ = tbl.set("loadlib", Value::Nil);
            let _ = tbl.set("loaders", Value::Nil);
            let _ = tbl.set("searchers", Value::Nil);
            let _ = globals.set("package", Value::Nil);
        }
        // Bare Lua `require` is intentionally nilled so plugins can't load
        // arbitrary code from disk. Plugins MUST use `host.require(name)`
        // (registered below) to access vendored Lua rocks. The `package`
        // table is nilled for the same reason; even if it weren't, its
        // `loaders` and `searchers` are stripped above.
        let _ = globals.set("require", Value::Nil);
        let instruction_limit = config.instruction_limit();
        let hook_interval = config.hook_interval();
        let _ = lua.set_hook(
            HookTriggers::new().every_nth_instruction(hook_interval),
            move |lua, _debug| {
                let globals = lua.globals();
                let count: i64 = globals.get("__livtet_instruction_count").unwrap_or(0);
                if count >= instruction_limit {
                    return Err(mlua::Error::external("plugin exceeded instruction limit"));
                }
                let _ = globals.set("__livtet_instruction_count", count + hook_interval as i64);
                Ok(VmState::Continue)
            },
        );
        Ok(lua)
    }

    fn setup_host_functions(&mut self) -> mlua::Result<()> {
        let host_table = self.lua.create_table()?;

        // -- HTTP transport --------------------------------------------------
        let host = Arc::clone(&self.host_impl);
        let http_get = self
            .lua
            .create_function(move |lua, args: mlua::MultiValue| {
                let mut iter = args.into_iter();
                let url: String = lua.from_value(iter.next().unwrap_or(Value::Nil))?;
                let opts = iter.next().unwrap_or(Value::Nil);
                let headers = extract_headers_from_opts(opts)?;
                let resp = host
                    .http_get(&url, &headers)
                    .map_err(|e| mlua::Error::external(e.to_string()))?;
                let t = lua.create_table()?;
                t.set("status", resp.status)?;
                t.set("body", resp.body)?;
                let hdrs = lua.create_table()?;
                for (k, v) in &resp.headers {
                    hdrs.set(k.as_str(), v.as_str())?;
                }
                t.set("headers", hdrs)?;
                Ok(t)
            })?;
        host_table.set("http_get", http_get)?;

        let host = Arc::clone(&self.host_impl);
        let http_post = self
            .lua
            .create_function(move |lua, args: mlua::MultiValue| {
                let mut iter = args.into_iter();
                let url: String = lua.from_value(iter.next().unwrap_or(Value::Nil))?;
                let body: Option<String> = match iter.next().unwrap_or(Value::Nil) {
                    Value::Nil => None,
                    v => Some(lua.from_value(v)?),
                };
                let opts = iter.next().unwrap_or(Value::Nil);
                let headers = extract_headers_from_opts(opts)?;
                let resp = host
                    .http_post(&url, body.as_deref(), &headers)
                    .map_err(|e| mlua::Error::external(e.to_string()))?;
                let t = lua.create_table()?;
                t.set("status", resp.status)?;
                t.set("body", resp.body)?;
                let hdrs = lua.create_table()?;
                for (k, v) in &resp.headers {
                    hdrs.set(k.as_str(), v.as_str())?;
                }
                t.set("headers", hdrs)?;
                Ok(t)
            })?;
        host_table.set("http_post", http_post)?;

        let host = Arc::clone(&self.host_impl);
        let http_put = self
            .lua
            .create_function(move |lua, args: mlua::MultiValue| {
                let mut iter = args.into_iter();
                let url: String = lua.from_value(iter.next().unwrap_or(Value::Nil))?;
                let body: Option<String> = match iter.next().unwrap_or(Value::Nil) {
                    Value::Nil => None,
                    v => Some(lua.from_value(v)?),
                };
                let opts = iter.next().unwrap_or(Value::Nil);
                let headers = extract_headers_from_opts(opts)?;
                let resp = host
                    .http_put(&url, body.as_deref(), &headers)
                    .map_err(|e| mlua::Error::external(e.to_string()))?;
                let t = lua.create_table()?;
                t.set("status", resp.status)?;
                t.set("body", resp.body)?;
                let hdrs = lua.create_table()?;
                for (k, v) in &resp.headers {
                    hdrs.set(k.as_str(), v.as_str())?;
                }
                t.set("headers", hdrs)?;
                Ok(t)
            })?;
        host_table.set("http_put", http_put)?;

        let host = Arc::clone(&self.host_impl);
        let http_patch = self
            .lua
            .create_function(move |lua, args: mlua::MultiValue| {
                let mut iter = args.into_iter();
                let url: String = lua.from_value(iter.next().unwrap_or(Value::Nil))?;
                let body: Option<String> = match iter.next().unwrap_or(Value::Nil) {
                    Value::Nil => None,
                    v => Some(lua.from_value(v)?),
                };
                let opts = iter.next().unwrap_or(Value::Nil);
                let headers = extract_headers_from_opts(opts)?;
                let resp = host
                    .http_patch(&url, body.as_deref(), &headers)
                    .map_err(|e| mlua::Error::external(e.to_string()))?;
                let t = lua.create_table()?;
                t.set("status", resp.status)?;
                t.set("body", resp.body)?;
                let hdrs = lua.create_table()?;
                for (k, v) in &resp.headers {
                    hdrs.set(k.as_str(), v.as_str())?;
                }
                t.set("headers", hdrs)?;
                Ok(t)
            })?;
        host_table.set("http_patch", http_patch)?;

        let host = Arc::clone(&self.host_impl);
        let http_delete = self
            .lua
            .create_function(move |lua, args: mlua::MultiValue| {
                let mut iter = args.into_iter();
                let url: String = lua.from_value(iter.next().unwrap_or(Value::Nil))?;
                let opts = iter.next().unwrap_or(Value::Nil);
                let headers = extract_headers_from_opts(opts)?;
                let resp = host
                    .http_delete(&url, &headers)
                    .map_err(|e| mlua::Error::external(e.to_string()))?;
                let t = lua.create_table()?;
                t.set("status", resp.status)?;
                t.set("body", resp.body)?;
                let hdrs = lua.create_table()?;
                for (k, v) in &resp.headers {
                    hdrs.set(k.as_str(), v.as_str())?;
                }
                t.set("headers", hdrs)?;
                Ok(t)
            })?;
        host_table.set("http_delete", http_delete)?;

        // -- log -------------------------------------------------------------
        let host = Arc::clone(&self.host_impl);
        let log = self.lua.create_function(
            move |_lua, (level, message): (String, String)| -> mlua::Result<()> {
                host.log("unknown", &level, &message);
                Ok(())
            },
        )?;
        host_table.set("log", log)?;

        // -- get_secret / set_secret -----------------------------------------
        let host = Arc::clone(&self.host_impl);
        let get_secret = self.lua.create_function(
            move |lua, (name,): (String,)| -> mlua::Result<Option<String>> {
                let plugin_id = read_current_plugin_id(lua);
                host.get_secret(&plugin_id, &name)
                    .map_err(|e| mlua::Error::external(e.to_string()))
            },
        )?;
        host_table.set("get_secret", get_secret)?;

        let host = Arc::clone(&self.host_impl);
        let set_secret = self.lua.create_function(
            move |lua, (name, value): (String, String)| -> mlua::Result<bool> {
                let plugin_id = read_current_plugin_id(lua);
                host.set_secret(&plugin_id, &name, &value)
                    .map_err(|e| mlua::Error::external(e.to_string()))?;
                Ok(true)
            },
        )?;
        host_table.set("set_secret", set_secret)?;

        // -- get_system_secret (compile-time SOPS secrets, grant-gated) ----
        //
        // Two-gate check enforced by the host:
        //   1. Manifest declares `system_secrets = true` in
        //      `[plugin.requires]` (populated via
        //      `declare_system_secrets` at plugin-load time).
        //   2. Grant sidecar allowlists the specific
        //      `PluginSystemSecret` being read.
        let plugin_grants = Arc::clone(&self.plugin_grants);
        let declared = Arc::clone(&self.system_secrets_declared);
        let host = Arc::clone(&self.host_impl);
        let perms_dir = permissions_dir();
        let get_system_secret = self.lua.create_function(
            move |lua, (name,): (String,)| -> mlua::Result<mlua::MultiValue> {
                let parsed = PluginSystemSecret::from_str(&name)
                    .map_err(|e| mlua::Error::runtime(format!("unknown system secret: {e:?}")))?;
                let plugin_id = read_current_plugin_id(lua);

                // Gate 1: manifest must declare system_secrets = true.
                {
                    let cache = declared
                        .lock()
                        .map_err(|e| mlua::Error::external(format!("declared cache: {e}")))?;
                    if !cache.get(&plugin_id).copied().unwrap_or(false) {
                        let err_msg =
                            "system secrets require 'system_secrets = true' in [plugin.requires]";
                        return Ok(mlua::MultiValue::from_vec(vec![
                            Value::Nil,
                            Value::String(lua.create_string(err_msg)?),
                        ]));
                    }
                }

                // Gate 2: grant sidecar allowlist.
                let grant = match load_or_cached_grant(&plugin_grants, &plugin_id, &perms_dir) {
                    Ok(Some(g)) => g,
                    Ok(None) => {
                        let msg = missing_sidecar_error(&plugin_id);
                        return Ok(mlua::MultiValue::from_vec(vec![
                            Value::Nil,
                            Value::String(lua.create_string(&msg)?),
                        ]));
                    }
                    Err(e) => {
                        let msg = format!("permission error: {e}");
                        return Ok(mlua::MultiValue::from_vec(vec![
                            Value::Nil,
                            Value::String(lua.create_string(&msg)?),
                        ]));
                    }
                };
                if !check_system_secret(&grant, parsed) {
                    let msg = system_secret_denied_error(&plugin_id, parsed);
                    return Ok(mlua::MultiValue::from_vec(vec![
                        Value::Nil,
                        Value::String(lua.create_string(&msg)?),
                    ]));
                }

                // Gates passed — resolve the secret.
                let value = host.get_system_secret(parsed);
                let v = match value {
                    Some(s) if !s.is_empty() => Value::String(lua.create_string(&s)?),
                    _ => Value::Nil,
                };
                Ok(mlua::MultiValue::from_vec(vec![v]))
            },
        )?;
        host_table.set("get_system_secret", get_system_secret)?;

        // -- url_encode / url_decode -----------------------------------------
        let host = Arc::clone(&self.host_impl);
        let url_encode =
            self.lua
                .create_function(move |_, (s,): (String,)| -> mlua::Result<String> {
                    Ok(host.url_encode(&s))
                })?;
        host_table.set("url_encode", url_encode)?;

        let host = Arc::clone(&self.host_impl);
        let url_decode =
            self.lua
                .create_function(move |_, (s,): (String,)| -> mlua::Result<String> {
                    host.url_decode(&s)
                        .map_err(|e| mlua::Error::external(e.to_string()))
                })?;
        host_table.set("url_decode", url_decode)?;

        // -- urn (in-process) ------------------------------------------
        // Builds a canonical URN string from a scheme + value. The
        // scheme must match `[%w_%-]+` so we don't emit strings the
        // Rust-side `Urn::parse` will reject; the value is passed
        // through verbatim (it may contain `:` or `/` since it's
        // the namespace-specific part). Validation is centralized
        // here so a Lua-side typo (e.g. `"urn:openlibrary" .. key`
        // instead of `"urn:openlibrary:" .. key`) fails loudly at
        // the plugin instead of silently emitting a string the
        // host later rejects with `UrnParseError::MissingSeparator`.
        let host = Arc::clone(&self.host_impl);
        let urn = self.lua.create_function(
            move |_, (ns, value): (String, String)| -> mlua::Result<String> {
                host.build_urn(&ns, &value)
                    .map_err(|e| mlua::Error::external(e.to_string()))
            },
        )?;
        host_table.set("urn", urn)?;

        // -- read_file (grant-gated) ----------------------------------------
        let grants = Arc::clone(&self.plugin_grants);
        let perms_dir = permissions_dir();
        let read_file = self.lua.create_function(
            move |lua, (path,): (String,)| -> mlua::Result<(Option<String>, Option<String>)> {
                let grant = load_grant!(&grants, &perms_dir, lua);
                let path_buf = camino::Utf8PathBuf::from(&path);
                if !check_read(&grant, &path_buf) {
                    let glob_hint = grant
                        .raw
                        .read_paths
                        .first()
                        .cloned()
                        .unwrap_or_else(|| "<empty>".to_string());
                    return Ok((None, Some(outside_glob_error(&path_buf, &glob_hint))));
                }
                match fs_err::read_to_string(&path_buf) {
                    Ok(content) => Ok((Some(content), None)),
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                        Ok((None, Some(format!("file not found: {path}"))))
                    }
                    Err(e) => Ok((None, Some(format!("read error: {e}")))),
                }
            },
        )?;
        host_table.set("read_file", read_file)?;

        // -- sqlite_query (grant-gated) -------------------------------------
        let grants = Arc::clone(&self.plugin_grants);
        let perms_dir = permissions_dir();
        let sqlite_query = self.lua.create_function(
            move |lua, (path, sql, params, limit): (String, String, Vec<Value>, Option<i64>)| {
                let grant = load_grant!(&grants, &perms_dir, lua);
                let path_buf = camino::Utf8PathBuf::from(&path);
                if !check_sqlite(&grant, &path_buf) {
                    let glob_hint = grant
                        .raw
                        .sqlite_paths
                        .first()
                        .cloned()
                        .unwrap_or_else(|| "<empty>".to_string());
                    return Ok((None, Some(outside_glob_error(&path_buf, &glob_hint))));
                }
                let params_json: Vec<serde_json::Value> = match params
                    .into_iter()
                    .map(|v| lua.from_value::<serde_json::Value>(v))
                    .collect::<mlua::Result<Vec<_>>>()
                {
                    Ok(p) => p,
                    Err(e) => return Ok((None, Some(format!("sqlite error: bad params: {e}")))),
                };
                let cap = limit.unwrap_or(10_000).clamp(1, 10_000) as usize;
                run_sqlite_query(lua, &path_buf, &sql, &params_json, cap)
            },
        )?;
        host_table.set("sqlite_query", sqlite_query)?;

        // -- fs_copy (grant-gated, returns __livtet_error on failure) ------
        //
        // `host.fs_copy(src, dst)` copies a file from `src` to `dst`.
        // Requires `src` ∈ `read_paths` AND `dst` ∈ `write_paths`.
        // Returns `true` on success or an `{ __livtet_error = { category,
        // message } }` table on failure. The Lua caller surfaces the
        // table to its own error envelope; the host never raises.
        let plugin_grants = Arc::clone(&self.plugin_grants);
        let perms_dir = permissions_dir();
        let fs_copy = self.lua.create_function(
            move |lua, (src, dst): (String, String)| -> mlua::Result<mlua::Value> {
                let plugin_id = read_current_plugin_id(lua);
                let grant = match load_or_cached_grant(&plugin_grants, &plugin_id, &perms_dir) {
                    Ok(Some(g)) => g,
                    Ok(None) => {
                        let err = lua.create_table()?;
                        err.set(
                            "__livtet_error",
                            lua.create_table_from([
                                ("category", "permission_denied".to_string()),
                                ("message", missing_sidecar_error(&plugin_id)),
                            ])?,
                        )?;
                        return Ok(mlua::Value::Table(err));
                    }
                    Err(e) => {
                        let err = lua.create_table()?;
                        err.set(
                            "__livtet_error",
                            lua.create_table_from([
                                ("category", "permission_denied".to_string()),
                                ("message", format!("permission error: {e}")),
                            ])?,
                        )?;
                        return Ok(mlua::Value::Table(err));
                    }
                };
                let src_path = camino::Utf8PathBuf::from(&src);
                let dst_path = camino::Utf8PathBuf::from(&dst);
                if !check_read(&grant, &src_path) {
                    let glob_hint = grant
                        .raw
                        .read_paths
                        .first()
                        .cloned()
                        .unwrap_or_else(|| "<empty>".to_string());
                    let err = lua.create_table()?;
                    err.set(
                        "__livtet_error",
                        lua.create_table_from([
                            ("category", "permission_denied".to_string()),
                            ("message", outside_glob_error(&src_path, &glob_hint)),
                        ])?,
                    )?;
                    return Ok(mlua::Value::Table(err));
                }
                if !check_write(&grant, &dst_path) {
                    let glob_hint = grant
                        .raw
                        .write_paths
                        .first()
                        .cloned()
                        .unwrap_or_else(|| "<empty>".to_string());
                    let err = lua.create_table()?;
                    err.set(
                        "__livtet_error",
                        lua.create_table_from([
                            ("category", "permission_denied".to_string()),
                            (
                                "message",
                                format!(
                                    "fs_copy: dst {dst:?} not in write_paths (hint: {glob_hint})"
                                ),
                            ),
                        ])?,
                    )?;
                    return Ok(mlua::Value::Table(err));
                }
                match fs_err::copy(&src, &dst) {
                    Ok(_) => Ok(mlua::Value::Boolean(true)),
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                        let err = lua.create_table()?;
                        err.set(
                            "__livtet_error",
                            lua.create_table_from([
                                ("category", "file_not_found".to_string()),
                                ("message", format!("fs_copy: {e}")),
                            ])?,
                        )?;
                        Ok(mlua::Value::Table(err))
                    }
                    Err(e) => {
                        let err = lua.create_table()?;
                        err.set(
                            "__livtet_error",
                            lua.create_table_from([
                                ("category", "file_error".to_string()),
                                ("message", format!("fs_copy: {e}")),
                            ])?,
                        )?;
                        Ok(mlua::Value::Table(err))
                    }
                }
            },
        )?;
        host_table.set("fs_copy", fs_copy)?;

        // -- fs_symlink (grant-gated, returns __livtet_error on failure) ---
        //
        // `host.fs_symlink(target, link_path)` creates a symlink at
        // `link_path` pointing at `target`. Requires `link_path` ∈
        // `write_paths`. The `target` arg is text only — file access
        // happens at the link, so we don't gate on it.
        let plugin_grants = Arc::clone(&self.plugin_grants);
        let perms_dir = permissions_dir();
        let fs_symlink = self.lua.create_function(
            move |lua, (target, link_path): (String, String)| -> mlua::Result<mlua::Value> {
                let plugin_id = read_current_plugin_id(lua);
                let grant = match load_or_cached_grant(&plugin_grants, &plugin_id, &perms_dir) {
                    Ok(Some(g)) => g,
                    Ok(None) => {
                        let err = lua.create_table()?;
                        err.set(
                            "__livtet_error",
                            lua.create_table_from([
                                (
                                    "category",
                                    "permission_denied".to_string(),
                                ),
                                (
                                    "message",
                                    missing_sidecar_error(&plugin_id),
                                ),
                            ])?,
                        )?;
                        return Ok(mlua::Value::Table(err));
                    }
                    Err(e) => {
                        let err = lua.create_table()?;
                        err.set(
                            "__livtet_error",
                            lua.create_table_from([
                                (
                                    "category",
                                    "permission_denied".to_string(),
                                ),
                                (
                                    "message",
                                    format!("permission error: {e}"),
                                ),
                            ])?,
                        )?;
                        return Ok(mlua::Value::Table(err));
                    }
                };
                let link = camino::Utf8PathBuf::from(&link_path);
                if !check_write(&grant, &link) {
                    let glob_hint = grant
                        .raw
                        .write_paths
                        .first()
                        .cloned()
                        .unwrap_or_else(|| "<empty>".to_string());
                    let err = lua.create_table()?;
                    err.set(
                        "__livtet_error",
                        lua.create_table_from([
                            (
                                "category",
                                "permission_denied".to_string(),
                            ),
                            (
                                "message",
                                format!(
                                    "fs_symlink: link_path {link_path:?} not in write_paths (hint: {glob_hint})"
                                ),
                            ),
                        ])?,
                    )?;
                    return Ok(mlua::Value::Table(err));
                }
                // `std::os::unix::fs::symlink` creates the link
                // without dereferencing the target, so a
                // non-existent `target` does NOT raise
                // `NotFound` here. We still keep the
                // `NotFound` arm below in case the link
                // path's parent directory is missing — that
                // surfaces as `ENOENT` on Unix and Windows
                // alike.
                #[cfg(not(unix))]
                {
                    let err = lua.create_table()?;
                    err.set(
                        "__livtet_error",
                        lua.create_table_from([
                            ("category", "file_error"),
                            (
                                "message",
                                "fs_symlink: not supported on this platform",
                            ),
                        ])?,
                    )?;
                    return Ok(mlua::Value::Table(err));
                }
                #[cfg(unix)]
                match std::os::unix::fs::symlink(&target, &link_path) {
                    Ok(_) => Ok(mlua::Value::Boolean(true)),
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                        let err = lua.create_table()?;
                        err.set(
                            "__livtet_error",
                            lua.create_table_from([
                                (
                                    "category",
                                    "file_not_found".to_string(),
                                ),
                                (
                                    "message",
                                    format!("fs_symlink: {e}"),
                                ),
                            ])?,
                        )?;
                        Ok(mlua::Value::Table(err))
                    }
                    Err(e) => {
                        let err = lua.create_table()?;
                        err.set(
                            "__livtet_error",
                            lua.create_table_from([
                                ("category", "file_error".to_string()),
                                (
                                    "message",
                                    format!("fs_symlink: {e}"),
                                ),
                            ])?,
                        )?;
                        Ok(mlua::Value::Table(err))
                    }
                }
            },
        )?;
        host_table.set("fs_symlink", fs_symlink)?;

        // -- emit_event (fire-and-forget) -----------------------------------
        let writer = ipc_writer_for_emit();
        let emit_event = self.lua.create_function(
            move |lua, (event_type, payload): (String, Value)| -> mlua::Result<()> {
                let plugin_id = read_current_plugin_id(lua);
                let payload_json: serde_json::Value = lua.from_value(payload)?;
                let req = HostToMain::EmitEvent {
                    plugin_id,
                    event_type,
                    payload: payload_json,
                };
                if let Some(w) = &writer {
                    use std::io::Write;
                    let payload = rmp_serde::to_vec_named(&req)
                        .map_err(|e| mlua::Error::external(e.to_string()))?;
                    let len = (payload.len() as u32).to_le_bytes();
                    if let Ok(mut guard) = w.lock() {
                        let _ = guard.write_all(&len);
                        let _ = guard.write_all(&payload);
                        let _ = guard.flush();
                    }
                }
                Ok(())
            },
        )?;
        host_table.set("emit_event", emit_event)?;

        // -- resolve_identifier / resolve_identifiers -----------------------
        let host = Arc::clone(&self.host_impl);
        let resolve_identifier = self.lua.create_function(
            move |lua, (urn,): (String,)| -> mlua::Result<Option<String>> {
                let _plugin_id = read_current_plugin_id(lua);
                host.resolve_identifier(&urn)
                    .map_err(|e| mlua::Error::external(e.to_string()))
            },
        )?;
        host_table.set("resolve_identifier", resolve_identifier)?;

        let host = Arc::clone(&self.host_impl);
        let resolve_identifiers = self.lua.create_function(
            move |lua, (urns,): (Vec<String>,)| -> mlua::Result<Value> {
                let _plugin_id = read_current_plugin_id(lua);
                let edition_ids = host
                    .resolve_identifiers(&urns)
                    .map_err(|e| mlua::Error::external(e.to_string()))?;
                let json: serde_json::Value = serde_json::Value::Array(
                    edition_ids
                        .into_iter()
                        .map(|e| match e {
                            Some(s) => serde_json::Value::String(s),
                            None => serde_json::Value::Null,
                        })
                        .collect(),
                );
                lua.to_value(&json)
            },
        )?;
        host_table.set("resolve_identifiers", resolve_identifiers)?;

        // -- get_edition_info (IPC) -----------------------------------------
        let host = Arc::clone(&self.host_impl);
        let get_edition_info = self.lua.create_function(
            move |lua, (edition_id,): (String,)| -> mlua::Result<Value> {
                let _plugin_id = read_current_plugin_id(lua);
                let info = host
                    .get_edition_info(&edition_id)
                    .map_err(|e| mlua::Error::external(e.to_string()))?;
                match info {
                    Some(v) => lua.to_value(&v),
                    None => Ok(Value::Nil),
                }
            },
        )?;
        host_table.set("get_edition_info", get_edition_info)?;

        let host = Arc::clone(&self.host_impl);
        let get_edition_identifiers = self.lua.create_function(
            move |lua, (edition_id,): (String,)| -> mlua::Result<Vec<String>> {
                let _plugin_id = read_current_plugin_id(lua);
                host.get_edition_identifiers(&edition_id)
                    .map_err(|e| mlua::Error::external(e.to_string()))
            },
        )?;
        host_table.set("get_edition_identifiers", get_edition_identifiers)?;

        // -- fetch_progress / upsert_progress (via traits) ------------------
        let host = Arc::clone(&self.host_impl);
        let fetch_progress = self.lua.create_function(
            move |lua, (urn,): (String,)| -> mlua::Result<Option<Value>> {
                let _plugin_id = read_current_plugin_id(lua);
                match host.fetch_progress(&urn) {
                    Ok(Some(entry)) => {
                        let json = serde_json::to_value(&entry)
                            .map_err(|e| mlua::Error::external(e.to_string()))?;
                        Ok(Some(lua.to_value(&json)?))
                    }
                    Ok(None) => Ok(None),
                    Err(e) => Err(mlua::Error::external(e.to_string())),
                }
            },
        )?;
        host_table.set("fetch_progress", fetch_progress)?;

        let host = Arc::clone(&self.host_impl);
        let upsert_progress = self.lua.create_function(
            move |lua,
                  (urn, progress, last_location, total_reading_time_secs): (
                String,
                f64,
                Option<String>,
                i64,
            )|
                  -> mlua::Result<Value> {
                let _plugin_id = read_current_plugin_id(lua);
                let result = host
                    .upsert_progress(&urn, progress, last_location, total_reading_time_secs)
                    .map_err(|e| mlua::Error::external(e.to_string()))?;
                lua.to_value(&result)
            },
        )?;
        host_table.set("upsert_progress", upsert_progress)?;

        // -- store_embedding (IPC, grant-gated) -----------------------------
        let grants = Arc::clone(&self.plugin_grants);
        let perms_dir = permissions_dir();
        let host = Arc::clone(&self.host_impl);
        let store_embedding = self.lua.create_function(
            move |lua, (edition_id, model, vector_hex): (String, String, String)| {
                let plugin_id = read_current_plugin_id(lua);
                let grant = match load_or_cached_grant(&grants, &plugin_id, &perms_dir) {
                    Ok(Some(g)) => g,
                    Ok(None) => {
                        return Err(mlua::Error::external(missing_sidecar_error(&plugin_id)));
                    }
                    Err(e) => {
                        return Err(mlua::Error::external(format!("permission error: {e}")));
                    }
                };
                if !check_embeddings(&grant) {
                    return Err(mlua::Error::external("embeddings permission not granted"));
                }
                let vector_bytes = hex::decode(&vector_hex)
                    .map_err(|e| mlua::Error::external(format!("invalid hex: {e}")))?;
                let result = host
                    .store_embedding(&edition_id, &model, &vector_bytes)
                    .map_err(|e| mlua::Error::external(e.to_string()))?;
                let tbl = lua.create_table()?;
                tbl.set("row_id", result.row_id)?;
                tbl.set("dimensions", result.dimensions)?;
                Ok(tbl)
            },
        )?;
        host_table.set("store_embedding", store_embedding)?;

        // -- get_embedding (IPC, grant-gated) -------------------------------
        let grants = Arc::clone(&self.plugin_grants);
        let perms_dir = permissions_dir();
        let host = Arc::clone(&self.host_impl);
        let get_embedding = self.lua.create_function(
            move |lua, (edition_id, model): (String, String)| -> mlua::Result<Value> {
                let plugin_id = read_current_plugin_id(lua);
                let grant = match load_or_cached_grant(&grants, &plugin_id, &perms_dir) {
                    Ok(Some(g)) => g,
                    Ok(None) => {
                        return Err(mlua::Error::external(missing_sidecar_error(&plugin_id)));
                    }
                    Err(e) => {
                        return Err(mlua::Error::external(format!("permission error: {e}")));
                    }
                };
                if !check_embeddings(&grant) {
                    return Err(mlua::Error::external("embeddings permission not granted"));
                }
                match host.get_embedding(&edition_id, &model) {
                    Ok(Some(resp)) => {
                        let tbl = lua.create_table()?;
                        tbl.set("vector", hex::encode(&resp.vector))?;
                        tbl.set("model", resp.model)?;
                        Ok(Value::Table(tbl))
                    }
                    Ok(None) => Ok(Value::Nil),
                    Err(e) => Err(mlua::Error::external(e.to_string())),
                }
            },
        )?;
        host_table.set("get_embedding", get_embedding)?;

        // -- find_similar_editions (IPC, grant-gated) ------------------------
        let grants = Arc::clone(&self.plugin_grants);
        let perms_dir = permissions_dir();
        let host = Arc::clone(&self.host_impl);
        let find_similar_editions = self.lua.create_function(
            move |lua, (query_vector_hex, model, limit): (String, String, usize)| {
                let plugin_id = read_current_plugin_id(lua);
                let grant = match load_or_cached_grant(&grants, &plugin_id, &perms_dir) {
                    Ok(Some(g)) => g,
                    Ok(None) => {
                        return Err(mlua::Error::external(missing_sidecar_error(&plugin_id)));
                    }
                    Err(e) => {
                        return Err(mlua::Error::external(format!("permission error: {e}")));
                    }
                };
                if !check_embeddings(&grant) {
                    return Err(mlua::Error::external("embeddings permission not granted"));
                }
                let query_vector = hex::decode(&query_vector_hex)
                    .map_err(|e| mlua::Error::external(format!("invalid hex: {e}")))?;
                let results = host
                    .find_similar_editions(&query_vector, &model, limit)
                    .map_err(|e| mlua::Error::external(e.to_string()))?;
                let tbl = lua.create_table()?;
                for (i, result) in results.into_iter().enumerate() {
                    let row = lua.create_table()?;
                    row.set("edition_id", result.edition_id)?;
                    row.set("score", result.score)?;
                    tbl.set(i + 1, row)?;
                }
                Ok(Value::Table(tbl))
            },
        )?;
        host_table.set("find_similar_editions", find_similar_editions)?;

        // -- has_plugin (in-process) ---------------------------------------
        let loaded_ids = Arc::clone(&self.loaded_ids);
        let has_plugin = self.lua.create_function(move |_, (id,): (String,)| {
            Ok(loaded_ids
                .lock()
                .map_err(|e| PluginError::MutexPoisoned(format!("loaded_ids: {e}")))?
                .contains(&id))
        })?;
        host_table.set("has_plugin", has_plugin)?;

        // -- plugin_dir (in-process) ---------------------------------------
        let plugin_dir = self
            .lua
            .create_function(|lua, ()| -> mlua::Result<Option<String>> {
                let dir: Option<String> = lua.globals().get(CURRENT_PLUGIN_DIR_GLOBAL)?;
                Ok(dir)
            })?;
        host_table.set("plugin_dir", plugin_dir)?;

        // -- plugin_asset (in-process) -------------------------------------
        let plugin_asset = self.lua.create_function(|lua, (filename,): (String,)| {
            let dir: Option<String> = lua.globals().get(CURRENT_PLUGIN_DIR_GLOBAL)?;
            let Some(dir) = dir else {
                return Err(mlua::Error::external(
                    "plugin_asset: plugin has no data directory",
                ));
            };
            let path = camino::Utf8Path::new(&dir).join("assets").join(&filename);
            let bytes = match fs_err::read(&path) {
                Ok(b) => b,
                Err(e) => return Err(mlua::Error::external(format!("read asset: {e}"))),
            };
            if bytes.contains(&0) {
                use base64::Engine;
                let encoded = base64::engine::general_purpose::STANDARD.encode(&bytes);
                Ok(Value::String(lua.create_string(&encoded)?))
            } else {
                match String::from_utf8(bytes.clone()) {
                    Ok(s) => Ok(Value::String(lua.create_string(&s)?)),
                    Err(_) => {
                        use base64::Engine;
                        let encoded = base64::engine::general_purpose::STANDARD.encode(&bytes);
                        Ok(Value::String(lua.create_string(&encoded)?))
                    }
                }
            }
        })?;
        host_table.set("plugin_asset", plugin_asset)?;

        // -- require (controlled loader) ------------------------------------
        // The Lua sandbox strips the global `require` at startup
        // (see `build_sandboxed_lua`), so plugins MUST go through
        // `host.require(name)`. Today this resolves the name
        // against the in-process `livtet-lua-stdlib` index: the
        // rock's source bytes are loaded as a Lua chunk and
        // executed, with the chunk's first return value handed
        // back to the caller.
        //
        // TODO: when the Tauri parent pre-installs rocks via
        // `luarocks` and exports `LUA_PATH` / `LUA_CPATH` to the
        // sidecar, extend this resolver to first try the bundled
        // index, then fall back to the parent-installed rock
        // tree. The fall-back requires re-exposing `package` (or
        // a sandboxed subset of it) so we can drive Lua's
        // `require` machinery; today's sandbox nulls `package`
        // out for safety. Tracking the cross-cutting design in
        // `docs/plans/in-progress/lua-http-and-luarocks.md`.
        let require_cache = Arc::clone(&self.require_cache);
        let require =
            self.lua
                .create_function(move |_lua, (target,): (String,)| -> mlua::Result<Value> {
                    if let Some(cached) = require_cache
                        .lock()
                        .ok()
                        .and_then(|g| g.get(&target).cloned())
                    {
                        return Ok(cached);
                    }
                    // TBD: the bundled Lua stdlib lookup (`livtet_lua_stdlib`)
                    // was removed when the empty stub crate was deleted.
                    // Wire a replacement that resolves `target` to either
                    // a vendored rocks source or a fallback error before
                    // re-enabling this code path. For now every
                    // `host.require` call returns an error so plugins
                    // fail fast rather than silently loading nothing.
                    Err(mlua::Error::external(format!(
                        "host.require: bundled-lua stdlib not wired (TBD); \
                         declare `rocks = [\"{target}\"]` in livtet.toml so \
                         the parent can install it via luarocks and expose \
                         it through LUA_PATH"
                    )))
                })?;
        host_table.set("require", require)?;

        // -- html_strip (in-process) ---------------------------------------
        let host = Arc::clone(&self.host_impl);
        let html_strip =
            self.lua
                .create_function(move |_, (html,): (String,)| -> mlua::Result<String> {
                    Ok(host.html_strip(&html))
                })?;
        host_table.set("html_strip", html_strip)?;

        // -- html_parse (in-process) ---------------------------------------
        let html_parse =
            self.lua
                .create_function(|lua, (html,): (String,)| -> mlua::Result<Value> {
                    let doc = ScraperHtml::parse_document(&html);
                    let userdata = lua.create_userdata(HtmlDoc { _html: doc })?;
                    Ok(Value::UserData(userdata))
                })?;
        host_table.set("html_parse", html_parse)?;

        // -- get_setting (in-process) -------------------------------------
        let plugin_settings = Arc::clone(&self.plugin_settings);
        let get_setting = self.lua.create_function(
            move |lua, (key,): (String,)| -> mlua::Result<Option<String>> {
                let plugin_id = read_current_plugin_id(lua);
                let map = plugin_settings
                    .lock()
                    .map_err(|e| PluginError::MutexPoisoned(format!("plugin_settings: {e}")))?;
                Ok(map.get(&plugin_id).and_then(|s| s.get(&key).cloned()))
            },
        )?;
        host_table.set("get_setting", get_setting)?;

        // -- set_setting --------------------------------------------------
        let host = Arc::clone(&self.host_impl);
        let plugin_settings_for_set = Arc::clone(&self.plugin_settings);
        let set_setting = self.lua.create_function(
            move |lua, (key, value): (String, String)| -> mlua::Result<bool> {
                let plugin_id = read_current_plugin_id(lua);
                host.set_setting(&plugin_id, &key, &value)
                    .map_err(|e| mlua::Error::external(e.to_string()))?;
                if let Ok(mut map) = plugin_settings_for_set.lock() {
                    let entry = map.entry(plugin_id).or_default();
                    entry.insert(key, value);
                }
                Ok(true)
            },
        )?;
        host_table.set("set_setting", set_setting)?;

        // -- oauth_redeem_token / oauth_get_token / oauth_revoke_token ----
        //
        // These three functions let a plugin run an OAuth redemption
        // flow against a third-party provider (typically
        // `livtet.olamaelcu.net`). All three are gated by:
        //
        //   1. The plugin's `[plugin.requires]` declares
        //      `oauth = true`.
        //   2. The grant sidecar at
        //      `<plugin_id>/permissions/<plugin_id>.toml` lists
        //      the requested `provider` in its `oauth_providers`
        //      allowlist. If the provider is not in the allowlist,
        //      the closure returns a `NEEDS_AUTH:<plugin_id>:<provider>`
        //      error string. The Tauri frontend matches this prefix
        //      to surface a "Grant access" prompt; the Rust IPC
        //      path uses the structured `HostError::NeedsAuth` for
        //      the same condition.
        //
        // The host runs the actual PKCE flow against the provider
        // — the plugin never touches raw authorization codes,
        // refresh tokens, or keyring entries. The host returns
        // only the access token (or an error) to the plugin.
        //
        // These are blocking IPC roundtrips. The host manager
        // dispatches the request to the main process, which opens
        // the system browser, waits for the user to complete the
        // consent UI, exchanges the code, and stores the grant. The
        // host's I/O thread blocks until the reply arrives (or the
        // request times out / is denied).
        //
        // All three closures return `(Option<String>, Option<String>)`
        // for symmetry — callers should treat the second slot as an
        // error message and the first slot as a result value. The
        // revoke function returns `Some("ok")` on success to match
        // the `(value, error)` shape used by every other OAuth host
        // function.
        let plugin_grants_for_oauth = Arc::clone(&self.plugin_grants);
        let perms_dir_for_oauth = permissions_dir();
        let host_oauth_redeem = Arc::clone(&self.host_impl);
        let oauth_redeem = self.lua.create_function(
            move |lua, (provider,): (String,)| -> mlua::Result<(Option<String>, Option<String>)> {
                let plugin_id = read_current_plugin_id(lua);
                let grant = match load_or_cached_grant(
                    &plugin_grants_for_oauth,
                    &plugin_id,
                    &perms_dir_for_oauth,
                ) {
                    Ok(Some(g)) => g,
                    Ok(None) => {
                        return Ok((None, Some(missing_sidecar_error(&plugin_id))));
                    }
                    Err(e) => {
                        return Ok((None, Some(format!("permission error: {e}"))));
                    }
                };
                if check_oauth(&grant, &provider).is_none() {
                    return Ok((None, Some(format!("NEEDS_AUTH:{}:{}", plugin_id, provider))));
                }
                match host_oauth_redeem.redeem_token(&plugin_id, &provider) {
                    Ok(token) => Ok((Some(token), None)),
                    Err(e) => Ok((None, Some(format!("oauth_redeem_token: {e}")))),
                }
            },
        )?;
        host_table.set("oauth_redeem_token", oauth_redeem)?;

        let plugin_grants_for_oauth_lookup = Arc::clone(&self.plugin_grants);
        let perms_dir_for_oauth_lookup = permissions_dir();
        let host_oauth_get_valid = Arc::clone(&self.host_impl);
        let oauth_get_token = self.lua.create_function(
            move |lua, (provider,): (String,)| -> mlua::Result<(Option<String>, Option<String>)> {
                let plugin_id = read_current_plugin_id(lua);
                let grant = match load_or_cached_grant(
                    &plugin_grants_for_oauth_lookup,
                    &plugin_id,
                    &perms_dir_for_oauth_lookup,
                ) {
                    Ok(Some(g)) => g,
                    Ok(None) => {
                        return Ok((None, Some(missing_sidecar_error(&plugin_id))));
                    }
                    Err(e) => {
                        return Ok((None, Some(format!("permission error: {e}"))));
                    }
                };
                if check_oauth(&grant, &provider).is_none() {
                    return Ok((None, Some(format!("NEEDS_AUTH:{}:{}", plugin_id, provider))));
                }
                match host_oauth_get_valid.get_valid_token(&plugin_id, &provider) {
                    Ok(token) => Ok((Some(token), None)),
                    Err(e) => Ok((None, Some(format!("oauth_get_token: {e}")))),
                }
            },
        )?;
        host_table.set("oauth_get_token", oauth_get_token)?;

        let plugin_grants_for_oauth_revoke = Arc::clone(&self.plugin_grants);
        let perms_dir_for_oauth_revoke = permissions_dir();
        let host_oauth_revoke = Arc::clone(&self.host_impl);
        let oauth_revoke_token = self.lua.create_function(
            move |lua, (provider,): (String,)| -> mlua::Result<(Option<String>, Option<String>)> {
                let plugin_id = read_current_plugin_id(lua);
                let grant = match load_or_cached_grant(
                    &plugin_grants_for_oauth_revoke,
                    &plugin_id,
                    &perms_dir_for_oauth_revoke,
                ) {
                    Ok(Some(g)) => g,
                    Ok(None) => {
                        return Ok((None, Some(missing_sidecar_error(&plugin_id))));
                    }
                    Err(e) => {
                        return Ok((None, Some(format!("permission error: {e}"))));
                    }
                };
                if check_oauth(&grant, &provider).is_none() {
                    return Ok((None, Some(format!("NEEDS_AUTH:{}:{}", plugin_id, provider))));
                }
                match host_oauth_revoke.revoke_token(&plugin_id, &provider) {
                    Ok(()) => Ok((Some("ok".to_string()), None)),
                    Err(e) => Ok((None, Some(format!("oauth_revoke_token: {e}")))),
                }
            },
        )?;
        host_table.set("oauth_revoke_token", oauth_revoke_token)?;

        // -- host.oauth_authorize(provider) --
        //
        // Fire-and-forget PKCE flow initiator. Opens the system browser
        // and registers the pending consent, but returns immediately.
        // The plugin calls `oauth_redeem_token` later to retrieve the
        // actual access token (once the user has completed the flow).
        let plugin_grants_for_oauth_authorize = Arc::clone(&self.plugin_grants);
        let perms_dir_for_oauth_authorize = permissions_dir();
        let host_oauth_authorize = Arc::clone(&self.host_impl);
        let oauth_authorize = self.lua.create_function(
            move |lua, (provider,): (String,)| -> mlua::Result<(Option<String>, Option<String>)> {
                let plugin_id = read_current_plugin_id(lua);
                let grant = match load_or_cached_grant(
                    &plugin_grants_for_oauth_authorize,
                    &plugin_id,
                    &perms_dir_for_oauth_authorize,
                ) {
                    Ok(Some(g)) => g,
                    Ok(None) => {
                        return Ok((None, Some(missing_sidecar_error(&plugin_id))));
                    }
                    Err(e) => {
                        return Ok((None, Some(format!("permission error: {e}"))));
                    }
                };
                if check_oauth(&grant, &provider).is_none() {
                    return Ok((None, Some(format!("NEEDS_AUTH:{}:{}", plugin_id, provider))));
                }
                match host_oauth_authorize.authorize(&plugin_id, &provider) {
                    Ok(()) => Ok((Some("ok".to_string()), None)),
                    Err(e) => Ok((None, Some(format!("oauth_authorize: {e}")))),
                }
            },
        )?;
        host_table.set("oauth_authorize", oauth_authorize)?;

        self.lua.globals().set("host", host_table)?;
        Ok(())
    }

    // FFI accessor added by SA9; do not use from inside the plugin crate
    // itself — only external hosts (FFI) need to override host functions
    // after construction. Returns a reference to the underlying Lua state
    // so callers (e.g. `livtet-ffi`) can register custom `host.require`
    // resolvers that bridge to the bundled-lua rock index.
    pub fn lua(&self) -> &Lua {
        &self.lua
    }

    pub fn handle_message(&mut self, msg: MainToHost) -> Option<HostToMain> {
        match msg {
            MainToHost::LoadPlugin {
                plugin_id,
                source,
                data_dir,
                settings,
                ..
            } => Some(self.load_plugin_source(&plugin_id, &source, data_dir, settings)),
            MainToHost::UnloadPlugin { plugin_id } => Some(self.unload_plugin(&plugin_id)),
            MainToHost::Call {
                id,
                plugin_id,
                capability,
                args,
            } => Some(self.call_capability(&id, &plugin_id, &capability, &args)),
            MainToHost::Shutdown => None,
        }
    }

    pub fn load_plugin_source(
        &mut self,
        id: &str,
        source: &str,
        data_dir: Option<camino::Utf8PathBuf>,
        settings: Option<std::collections::HashMap<String, String>>,
    ) -> HostToMain {
        let _ = self
            .lua
            .globals()
            .set(CURRENT_PLUGIN_ID_GLOBAL, id.to_string());
        match data_dir {
            Some(ref dir) => {
                let _ = self
                    .lua
                    .globals()
                    .set(CURRENT_PLUGIN_DIR_GLOBAL, dir.to_string());
            }
            None => {
                let _ = self
                    .lua
                    .globals()
                    .set(CURRENT_PLUGIN_DIR_GLOBAL, Value::Nil);
            }
        }
        let settings = settings.unwrap_or_default();
        match self.lua.load(source).eval::<Value>() {
            Ok(Value::Table(provider)) => {
                self.loaded_plugins
                    .insert(id.to_string(), PluginEntry { provider, data_dir });
                let mut ids = match self.loaded_ids.lock() {
                    Ok(guard) => guard,
                    Err(e) => {
                        return HostToMain::PluginLoadError {
                            plugin_id: id.to_string(),
                            error: format!("mutex poisoned: {e}"),
                        };
                    }
                };
                ids.insert(id.to_string());
                drop(ids);
                let mut settings_lock = match self.plugin_settings.lock() {
                    Ok(guard) => guard,
                    Err(e) => {
                        return HostToMain::PluginLoadError {
                            plugin_id: id.to_string(),
                            error: format!("mutex poisoned: {e}"),
                        };
                    }
                };
                settings_lock.insert(id.to_string(), settings);
                drop(settings_lock);
                HostToMain::PluginLoaded {
                    plugin_id: id.to_string(),
                    load_state: "loaded".to_string(),
                    missing_optional: Vec::new(),
                }
            }
            Ok(_) => HostToMain::PluginLoadError {
                plugin_id: id.to_string(),
                error: "plugin must return a table".to_string(),
            },
            Err(e) => HostToMain::PluginLoadError {
                plugin_id: id.to_string(),
                error: e.to_string(),
            },
        }
    }

    fn unload_plugin(&mut self, id: &str) -> HostToMain {
        self.loaded_plugins.remove(id);
        if let Ok(mut ids) = self.loaded_ids.lock() {
            ids.remove(id);
        }
        if let Ok(mut settings) = self.plugin_settings.lock() {
            settings.remove(id);
        }
        HostToMain::PluginUnloaded {
            plugin_id: id.to_string(),
        }
    }

    pub fn call_capability(
        &mut self,
        id: &str,
        plugin_id: &str,
        capability: &str,
        args: &[serde_json::Value],
    ) -> HostToMain {
        let _ = self
            .lua
            .globals()
            .set(CURRENT_PLUGIN_ID_GLOBAL, plugin_id.to_string());
        let _ = self
            .lua
            .globals()
            .set(CURRENT_CALL_ID_GLOBAL, id.to_string());
        let data_dir = self
            .loaded_plugins
            .get(plugin_id)
            .and_then(|e| e.data_dir.as_ref())
            .map(|d| d.to_string());
        match data_dir {
            Some(ref dir) => {
                let _ = self
                    .lua
                    .globals()
                    .set(CURRENT_PLUGIN_DIR_GLOBAL, dir.clone());
            }
            None => {
                let _ = self
                    .lua
                    .globals()
                    .set(CURRENT_PLUGIN_DIR_GLOBAL, Value::Nil);
            }
        }

        let func: Function = match self.loaded_plugins.get(plugin_id) {
            Some(entry) => match entry.provider.get(capability) {
                Ok(f) => f,
                Err(_) => {
                    return HostToMain::CallResult {
                        id: id.to_string(),
                        ok: false,
                        value: None,
                        error: Some(format!(
                            "capability '{capability}' not found on plugin '{plugin_id}'"
                        )),
                    };
                }
            },
            None => {
                return HostToMain::CallResult {
                    id: id.to_string(),
                    ok: false,
                    value: None,
                    error: Some(format!("plugin not found: {plugin_id}")),
                };
            }
        };

        let args_str = args
            .iter()
            .map(json_to_lua_literal)
            .collect::<Vec<_>>()
            .join(", ");

        if let Err(e) = self.lua.globals().set("__call_target", func) {
            return HostToMain::CallResult {
                id: id.to_string(),
                ok: false,
                value: None,
                error: Some(format!("failed to install call target: {e}")),
            };
        }

        let code =
            format!("local __f = __call_target; __call_target = nil; return __f({args_str})");

        let result = match self.lua.load(&code).eval::<Value>() {
            Ok(v) => v,
            Err(e) => {
                return HostToMain::CallResult {
                    id: id.to_string(),
                    ok: false,
                    value: None,
                    error: Some(e.to_string()),
                };
            }
        };

        let json_value: serde_json::Value = match self.lua.from_value(result) {
            Ok(v) => v,
            Err(e) => {
                return HostToMain::CallResult {
                    id: id.to_string(),
                    ok: false,
                    value: None,
                    error: Some(format!("result conversion: {e}")),
                };
            }
        };

        HostToMain::CallResult {
            id: id.to_string(),
            ok: true,
            value: Some(json_value),
            error: None,
        }
    }
}

struct DefaultSandbox;
impl SandboxConfig for DefaultSandbox {}

struct HtmlDoc {
    _html: ScraperHtml,
}

impl UserData for HtmlDoc {
    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        methods.add_method("select", |_lua, this, sel: String| {
            let selector =
                Selector::parse(&sel).map_err(|e| mlua::Error::external(e.to_string()))?;
            let table = _lua.create_table()?;
            for (idx, element) in (1usize..).zip(this._html.select(&selector)) {
                let text = element
                    .text()
                    .collect::<Vec<_>>()
                    .join("")
                    .trim()
                    .to_string();
                let mut attrs: HashMap<String, String> = HashMap::new();
                for (name, value) in element.value().attrs() {
                    attrs.insert(name.to_string(), value.to_string());
                }
                let el_userdata = _lua.create_userdata(ElementHandle { text, attrs })?;
                table.set(idx, el_userdata)?;
            }
            Ok(table)
        });
    }
}

struct ElementHandle {
    text: String,
    attrs: HashMap<String, String>,
}

impl UserData for ElementHandle {
    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        methods.add_method("text", |_lua, this, ()| -> mlua::Result<String> {
            Ok(this.text.clone())
        });
        methods.add_method(
            "attr",
            |_lua, this, name: String| -> mlua::Result<Option<String>> {
                Ok(this.attrs.get(&name).cloned())
            },
        );
    }
}

// ── Free functions ─────────────────────────────────────────────────────────

fn read_current_plugin_id(lua: &Lua) -> String {
    lua.globals()
        .get::<Option<String>>(CURRENT_PLUGIN_ID_GLOBAL)
        .ok()
        .flatten()
        .unwrap_or_else(|| "unknown".to_string())
}

fn extract_headers_from_opts(opts: Value) -> mlua::Result<Vec<(String, String)>> {
    if let Value::Table(t) = opts {
        let headers_val: Value = t.get("headers").unwrap_or(Value::Nil);
        if let Value::Table(headers_t) = headers_val {
            let mut headers = Vec::new();
            for pair in headers_t.pairs::<String, String>() {
                let (k, v) = pair?;
                headers.push((k, v));
            }
            return Ok(headers);
        }
    }
    Ok(Vec::new())
}

/// Serialize a serde_json::Value to a Lua literal string for
/// embedding in dynamically-generated Lua code.
fn json_to_lua_literal(v: &serde_json::Value) -> String {
    match v {
        serde_json::Value::Null => "nil".to_string(),
        serde_json::Value::Bool(b) => {
            if *b {
                "true".to_string()
            } else {
                "false".to_string()
            }
        }
        serde_json::Value::Number(n) => n.to_string(),
        serde_json::Value::String(s) => {
            let escaped = s
                .replace('\\', "\\\\")
                .replace('"', "\\\"")
                .replace('\n', "\\n")
                .replace('\r', "\\r")
                .replace('\t', "\\t");
            format!("\"{escaped}\"")
        }
        serde_json::Value::Array(arr) => {
            let items: Vec<String> = arr.iter().map(json_to_lua_literal).collect();
            format!("{{ {} }}", items.join(", "))
        }
        serde_json::Value::Object(obj) => {
            let items: Vec<String> = obj
                .iter()
                .map(|(k, v)| {
                    let k_lit = serde_json::Value::String(k.clone());
                    format!(
                        "[{}] = {}",
                        json_to_lua_literal(&k_lit),
                        json_to_lua_literal(v)
                    )
                })
                .collect();
            format!("{{ {} }}", items.join(", "))
        }
    }
}

fn run_sqlite_query(
    lua: &Lua,
    path: &camino::Utf8Path,
    sql: &str,
    _params: &[serde_json::Value],
    cap: usize,
) -> mlua::Result<(Option<mlua::Value>, Option<String>)> {
    use livtet_data::sql::{
        Column as _, Row,
        sqlite::{SqliteConnectOptions, SqliteRow},
    };

    let rt = tokio::runtime::Runtime::new().map_err(|e| mlua::Error::external(e.to_string()))?;

    let opts = SqliteConnectOptions::new()
        .filename(path.as_std_path())
        .read_only(true);
    let pool = rt.block_on(async {
        livtet_core::sqlite_pool_options()
            .max_connections(1)
            .connect_with(opts)
            .await
    });
    let pool = match pool {
        Ok(p) => p,
        Err(e) => {
            return Ok((None, Some(format!("sqlite connect: {e}"))));
        }
    };

    let rows_result: Result<Vec<SqliteRow>, _> =
        rt.block_on(async { livtet_data::sql::query(AssertSqlSafe(sql)).fetch_all(&pool).await });
    let rows = match rows_result {
        Ok(r) => r,
        Err(e) => {
            return Ok((None, Some(format!("sqlite query: {e}"))));
        }
    };

    let table = lua
        .create_table()
        .map_err(|e| mlua::Error::external(e.to_string()))?;

    let columns: Vec<String> = if !rows.is_empty() {
        rows[0]
            .columns()
            .iter()
            .map(|c| c.name().to_string())
            .collect()
    } else {
        Vec::new()
    };

    let cols_table = lua
        .create_table()
        .map_err(|e| mlua::Error::external(e.to_string()))?;
    for (i, col) in columns.iter().enumerate() {
        cols_table
            .set(i + 1, col.as_str())
            .map_err(|e| mlua::Error::external(e.to_string()))?;
    }
    table
        .set("columns", cols_table)
        .map_err(|e| mlua::Error::external(e.to_string()))?;

    let rows_table = lua
        .create_table()
        .map_err(|e| mlua::Error::external(e.to_string()))?;
    for (idx, row) in rows.iter().enumerate() {
        if idx >= cap {
            break;
        }
        let row_table = lua
            .create_table()
            .map_err(|e| mlua::Error::external(e.to_string()))?;
        for (col_idx, col) in columns.iter().enumerate() {
            let int_val: Option<i64> = row.try_get(col_idx).ok();
            let real_val: Option<f64> = row.try_get(col_idx).ok();
            let text_val: Option<&str> = row.try_get(col_idx).ok();
            let blob_val: Option<&[u8]> = row.try_get(col_idx).ok();
            let lua_val = if let Some(v) = int_val {
                Value::Integer(v)
            } else if let Some(v) = real_val {
                Value::Number(mlua::Number::from(v))
            } else if let Some(v) = text_val {
                Value::String(
                    lua.create_string(v)
                        .map_err(|e| mlua::Error::external(e.to_string()))?,
                )
            } else if let Some(v) = blob_val {
                Value::String(
                    lua.create_string(hex::encode(v))
                        .map_err(|e| mlua::Error::external(e.to_string()))?,
                )
            } else {
                Value::Nil
            };
            row_table
                .set(col.as_str(), lua_val)
                .map_err(|e| mlua::Error::external(e.to_string()))?;
        }
        rows_table
            .set(idx + 1, row_table)
            .map_err(|e| mlua::Error::external(e.to_string()))?;
    }
    table
        .set("rows", rows_table)
        .map_err(|e| mlua::Error::external(e.to_string()))?;

    Ok((Some(Value::Table(table)), None))
}

/// Return a shared writer for fire-and-forget IPC messages.
/// Returns `None` when the globals haven't been set up — the
/// caller should degrade gracefully.
fn ipc_writer_for_emit() -> Option<Arc<Mutex<Box<dyn std::io::Write + Send>>>> {
    None
}

fn load_or_cached_grant(
    cache: &PluginGrants,
    plugin_id: &str,
    permissions_dir: &camino::Utf8Path,
) -> Result<Option<Arc<ResolvedGrant>>, PluginError> {
    {
        let map = cache
            .lock()
            .map_err(|e| PluginError::MutexPoisoned(format!("grants cache: {e}")))?;
        if let Some(entry) = map.get(plugin_id) {
            return Ok(entry.clone());
        }
    }
    let loaded = load_grant(plugin_id, permissions_dir)?;
    let mut map = cache
        .lock()
        .map_err(|e| PluginError::MutexPoisoned(format!("grants cache: {e}")))?;
    map.insert(plugin_id.to_string(), loaded.clone());
    Ok(loaded)
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use globset::GlobSetBuilder;

    use super::*;
    use crate::{
        embedded_host::EmbeddedHost,
        permissions::{PluginGrant, ResolvedGrant},
        system_secrets::PluginSystemSecret,
    };

    /// Build a `ResolvedGrant` with the given allowlist of system
    /// secrets. Test-only helper: production code goes through the
    /// sidecar loader.
    fn resolved_grant_for_system_secrets(allowed: &[PluginSystemSecret]) -> Arc<ResolvedGrant> {
        let raw_strings: Vec<String> = allowed.iter().map(|s| s.as_ref().to_string()).collect();
        Arc::new(ResolvedGrant {
            raw: PluginGrant {
                version: 1,
                read_paths: Vec::new(),
                sqlite_paths: Vec::new(),
                allow_writes: false,
                write_paths: Vec::new(),
                system_secrets: raw_strings,
                embeddings: false,
                oauth_providers: Vec::new(),
                http_proxy_url: None,
            },
            read_paths: GlobSetBuilder::new().build().expect("empty globset"),
            sqlite_paths: GlobSetBuilder::new().build().expect("empty globset"),
            write_paths: GlobSetBuilder::new().build().expect("empty globset"),
            system_secrets: allowed.iter().copied().collect(),
            embeddings: false,
            oauth_providers: std::collections::HashMap::new(),
            http_proxy_url: None,
        })
    }

    #[test]
    fn host_get_system_secret_returns_registered_value_when_gated() {
        let mut secrets = std::collections::HashMap::new();
        secrets.insert(
            PluginSystemSecret::GoogleBooksApiKey,
            "AIza-test-key".to_string(),
        );
        let embedded = EmbeddedHost::with_system_secrets(secrets);
        let host = LuaHost::new(Arc::new(embedded)).expect("LuaHost::new");

        // Set plugin id so the bridge picks it up via
        // read_current_plugin_id (otherwise the host defaults to
        // "unknown" and the gate rejects the call).
        host.lua
            .globals()
            .set(CURRENT_PLUGIN_ID_GLOBAL, "test_plugin")
            .expect("set plugin id");

        // Gate 1: declare the manifest-side capability.
        host.declare_system_secrets("test_plugin", true);
        // Gate 2: pre-populate the grants cache (bypasses the
        // sidecar file system so this test is hermetic).
        host.plugin_grants
            .lock()
            .expect("plugin_grants lock")
            .insert(
                "test_plugin".to_string(),
                Some(resolved_grant_for_system_secrets(&[
                    PluginSystemSecret::GoogleBooksApiKey,
                ])),
            );

        let result: Option<String> = host
            .lua
            .load(r#"return host.get_system_secret("google_books_api_key")"#)
            .eval()
            .expect("eval should succeed for known variant");
        assert_eq!(
            result,
            Some("AIza-test-key".to_string()),
            "known variant should return registered value",
        );
    }

    /// Gate 1: a plugin that does not declare `system_secrets = true`
    /// in `[plugin.requires]` is rejected before the grant sidecar
    /// is consulted. The Lua side sees `nil` and the canonical
    /// "manifest missing the declaration" error.
    #[test]
    fn host_get_system_secret_gate1_rejects_plugin_without_manifest_declaration() {
        let mut secrets = std::collections::HashMap::new();
        secrets.insert(
            PluginSystemSecret::GoogleBooksApiKey,
            "AIza-test-key".to_string(),
        );
        let embedded = EmbeddedHost::with_system_secrets(secrets);
        let host = LuaHost::new(Arc::new(embedded)).expect("LuaHost::new");

        host.lua
            .globals()
            .set(CURRENT_PLUGIN_ID_GLOBAL, "ungated_plugin")
            .expect("set plugin id");

        // Note: NOT calling declare_system_secrets, so Gate 1 fails.
        // Gate 2: even with a fully-granted plugin, the bridge still
        // returns nil + error because Gate 1 trips first.
        host.plugin_grants
            .lock()
            .expect("plugin_grants lock")
            .insert(
                "ungated_plugin".to_string(),
                Some(resolved_grant_for_system_secrets(&[
                    PluginSystemSecret::GoogleBooksApiKey,
                ])),
            );

        let (value, err): (Option<String>, Option<String>) = host
            .lua
            .load(r#"return host.get_system_secret("google_books_api_key")"#)
            .eval()
            .expect("eval should succeed; gate failure becomes a Lua return");
        assert!(value.is_none(), "expected nil value, got {value:?}");
        let err_msg = err.expect("expected an error message from Gate 1");
        assert!(
            err_msg.contains("system_secrets = true"),
            "expected gate-1 error message; got: {err_msg}",
        );
    }

    /// Gate 2: a plugin that declares the capability but is NOT
    /// allowlisted for the specific secret in its grant sidecar gets
    /// the canonical "permission denied" error from
    /// `system_secret_denied_error`.
    #[test]
    fn host_get_system_secret_gate2_rejects_unlisted_secret() {
        let embedded = EmbeddedHost::new();
        let host = LuaHost::new(Arc::new(embedded)).expect("LuaHost::new");

        host.lua
            .globals()
            .set(CURRENT_PLUGIN_ID_GLOBAL, "listed_plugin")
            .expect("set plugin id");
        host.declare_system_secrets("listed_plugin", true);
        // Grant allowlists `platform_unauthenticated_allowed` but
        // not `google_books_api_key` — the test asks for the latter.
        host.plugin_grants
            .lock()
            .expect("plugin_grants lock")
            .insert(
                "listed_plugin".to_string(),
                Some(resolved_grant_for_system_secrets(&[
                    PluginSystemSecret::PlatformUnauthenticatedAllowed,
                ])),
            );

        let (value, err): (Option<String>, Option<String>) = host
            .lua
            .load(r#"return host.get_system_secret("google_books_api_key")"#)
            .eval()
            .expect("eval should succeed; gate failure is a Lua return");
        assert!(value.is_none(), "expected nil value, got {value:?}");
        let err_msg = err.expect("expected gate-2 error message");
        assert!(
            err_msg.contains("permission denied"),
            "expected permission-denied message; got: {err_msg}",
        );
        assert!(
            err_msg.contains("google_books_api_key"),
            "error should name the missing secret; got: {err_msg}",
        );
    }

    #[test]
    fn host_get_system_secret_raises_error_for_unknown_name() {
        let embedded = EmbeddedHost::new();
        let host = LuaHost::new(Arc::new(embedded)).expect("LuaHost::new");

        let result: mlua::Result<Option<String>> = host
            .lua
            .load(r#"return host.get_system_secret("totally_not_a_variant")"#)
            .eval();
        match result {
            Err(e) => {
                let msg = e.to_string();
                assert!(
                    msg.contains("unknown system secret"),
                    "error message should contain 'unknown system secret', got: {msg}",
                );
            }
            Ok(_) => panic!("expected error for unknown secret name, but got Ok"),
        }
    }
}
