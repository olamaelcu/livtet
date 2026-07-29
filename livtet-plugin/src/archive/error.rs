use camino::Utf8PathBuf;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ArchiveError {
    #[error("invalid archive: {0}")]
    InvalidArchive(String),

    #[error("missing META-INF file: {0}")]
    MissingMetadata(String),

    #[error("signature verification failed")]
    InvalidSignature,

    #[error("integrity check failed: {path}")]
    IntegrityCheckFailed { path: String },

    #[error("unsigned file in archive: {0}")]
    UnsignedFile(String),

    #[error("manifest mismatch on field: {field}")]
    ManifestMismatch { field: String },

    #[error("archive signed by revoked key")]
    RevokedKey,

    #[error("archive signed by untrusted key: {fingerprint}")]
    UntrustedKey { fingerprint: String },

    #[error("unsupported archive format version: {version}")]
    UnsupportedFormat { version: u32 },

    #[error("plugin {id} v{version} already installed")]
    AlreadyInstalled { id: String, version: String },

    #[error("installation failed: {0}")]
    InstallationFailed(String),

    #[error("no signing key found with label '{label}'")]
    NoSigningKey { label: String },

    #[error("passphrase required for key at {key_path} but none provided")]
    PassphraseRequired { key_path: Utf8PathBuf },

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("zip error: {0}")]
    Zip(#[from] zip::result::ZipError),

    #[error("key error: {0}")]
    Key(String),

    #[error("time error: {0}")]
    Time(#[from] time::Error),
}
