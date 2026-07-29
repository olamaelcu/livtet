use thiserror::Error;

#[derive(Debug, Error)]
pub enum RepositoryError {
    #[error("repository not found: {0}")]
    NotFound(String),

    #[error("repository already added: {0}")]
    AlreadyAdded(String),

    #[error("network error: {0}")]
    Network(String),

    #[error("HTTP error {status} at {url}")]
    Http { status: u16, url: String },

    #[error("index.json signature verification failed")]
    BadIndexSignature,

    #[error("index.json parse error: {0}")]
    IndexParse(String),

    #[error("plugin not found in index: {0}")]
    PluginNotFound(String),

    #[error("version not found for plugin {id}: {version}")]
    VersionNotFound { id: String, version: String },

    #[error("archive download failed: {0}")]
    DownloadFailed(String),

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("HMAC error: {0}")]
    Hmac(String),

    #[error("keyring error: {0}")]
    Keyring(String),

    #[error(transparent)]
    Archive(#[from] crate::archive::error::ArchiveError),
}
