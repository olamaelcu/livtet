pub use mlua;

pub mod annotations;
pub mod archive;
pub mod capability;
pub mod discovery;
pub mod embedded_host;
pub mod error;
pub mod host_lua;
pub mod host_manager;
pub mod host_trait;
pub mod import;
pub mod ipc_host;
pub mod keys;
pub mod link_resolver;
pub mod luarocks;
pub mod manifest;
pub mod permissions;
pub mod plugin_requires;
pub mod progress_entry;
pub mod protocol;
pub mod pull;
pub mod reading_list;
pub mod reading_progress;
pub mod repository;
pub mod series;
pub mod system_secrets;
pub mod transport;
pub mod types;
pub mod watch;
pub mod web_registry;

pub use error::{PluginError, PluginResult};
pub use host_manager::{CommandEnv, HostSpawnConfig, PluginHostManager};
pub use manifest::{
    PluginManifest, PluginMeta, PluginRuntime, PluginType, SettingFieldType, SettingSchema,
};
pub use permissions::{
    GrantFormat, OAuthGrantEntry, PluginGrant, ResolvedGrant, check_http_proxy, check_read,
    check_sqlite, default_grant_path, default_permissions_dir, http_proxy_denied_error, load_grant,
    missing_sidecar_error, outside_glob_error, permissions_dir,
};
pub use pull::{EnrichedPullEntry, PullResult, RawPullEntry};
pub use web_registry::{
    SlotContribution, WebContributionRegistry, WebContributions, parse_web_contributions,
};

pub const HOST_LUA_BINARY_NAME: &str = if cfg!(windows) {
    "livtet-plugins-host-lua.exe"
} else {
    "livtet-plugins-host-lua"
};
