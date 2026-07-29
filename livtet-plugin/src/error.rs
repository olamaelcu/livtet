#[derive(Debug, thiserror::Error)]
pub enum PluginError {
    #[error("manifest parse error: {0}")]
    ManifestParse(#[from] toml::de::Error),

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("ipc error: {0}")]
    Ipc(String),

    #[error("plugin not found: {0}")]
    PluginNotFound(String),

    #[error("plugin load failed: {id}: {error}")]
    PluginLoadFailed { id: String, error: String },

    #[error("host crashed: {0}")]
    HostCrashed(String),

    #[error("call timeout: {0}")]
    Timeout(String),

    #[error("discovery error: {0}")]
    Discovery(String),

    #[error("archive error: {0}")]
    Archive(#[from] crate::archive::error::ArchiveError),

    #[error("serialization error: {0}")]
    Serialization(String),

    #[error("repository error: {0}")]
    Repository(#[from] crate::repository::error::RepositoryError),

    #[error("lua error: {0}")]
    Lua(String),

    #[error("mutex poisoned: {0}")]
    MutexPoisoned(String),

    #[error("luarocks error: {0}")]
    Luarocks(String),
}

pub type PluginResult<T> = Result<T, PluginError>;

impl From<PluginError> for mlua::Error {
    fn from(e: PluginError) -> Self {
        mlua::Error::external(e)
    }
}

impl From<mlua::Error> for PluginError {
    fn from(e: mlua::Error) -> Self {
        PluginError::Lua(e.to_string())
    }
}
