use miette::Diagnostic;
use thiserror::Error;

#[derive(Error, Debug, Diagnostic)]
#[allow(clippy::disallowed_types)]
pub enum CliError {
    #[error("I/O error: {0}")]
    #[diagnostic(code(livtet_cli::io))]
    Io(#[from] std::io::Error),

    #[error("Serde JSON error: {0}")]
    #[diagnostic(code(livtet_cli::serde_json))]
    SerdeJson(#[from] serde_json::Error),

    #[error("Plugin subsystem error: {0}")]
    #[diagnostic(code(livtet_cli::plugin))]
    Plugin(#[from] livtet_plugins::PluginError),

    #[error("Repository error: {0}")]
    #[diagnostic(code(livtet_cli::repository))]
    Repository(#[from] livtet_plugins::repository::error::RepositoryError),

    #[error("Archive error: {0}")]
    #[diagnostic(code(livtet_cli::archive))]
    Archive(#[from] livtet_plugins::archive::error::ArchiveError),

    #[error("Passphrase is empty; refusing to derive a 32-byte key from nothing")]
    #[diagnostic(code(livtet_cli::keyring_recover_empty_passphrase))]
    EmptyPassphrase,

    #[error(
        "no passphrase source specified: pass --passphrase (for testing only), \
         --passphrase-stdin (for scripting), or --interactive (for a TTY prompt)"
    )]
    #[help("Re-run with --interactive to be prompted via `inquire::Password`.")]
    #[diagnostic(code(livtet_cli::keyring_recover_no_source))]
    NoPassphraseSource,

    #[error(
        "OS keyring unavailable and no passphrase recovery file at {path}.\n\
         Recover access with: livtet keyring-recover --passphrase-stdin\n\
         (the passphrase you used originally will reproduce the same HMAC key)."
    )]
    #[diagnostic(code(livtet_cli::hmac_no_source))]
    NoHmacSource { path: std::path::PathBuf },

    #[error(
        "recovery file at {path} is malformed: expected a single line starting with \
         LIVTET_PASSPHRASE_HMAC_KEY="
    )]
    #[diagnostic(code(livtet_cli::keyring_recover_malformed))]
    RecoveryFileMalformed { path: std::path::PathBuf },

    #[error("recovery file at {path} has invalid hex")]
    #[diagnostic(code(livtet_cli::keyring_recover_invalid_hex))]
    RecoveryFileInvalidHex {
        path: std::path::PathBuf,
        #[source]
        source: hex::FromHexError,
    },

    #[error("recovery file at {path} has wrong size: {actual} bytes (expected 32)")]
    #[diagnostic(code(livtet_cli::keyring_recover_wrong_size))]
    RecoveryFileWrongSize {
        path: std::path::PathBuf,
        actual: usize,
    },

    #[error("failed to write recovery file to {path}")]
    #[diagnostic(code(livtet_cli::keyring_recover_write))]
    RecoveryFileWrite {
        path: std::path::PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("signing key changed; confirm-update required")]
    #[help("Run `livtet plugin trust <path-to-pubkey>` then `livtet repo confirm-update <name>`.")]
    #[diagnostic(code(livtet_cli::signing_key_changed))]
    SigningKeyChanged,

    #[error("archive failed verification: {errors:?}")]
    #[diagnostic(code(livtet_cli::archive_verification_failed))]
    ArchiveVerificationFailed { errors: Vec<String> },

    #[error("verify report missing plugin_id")]
    #[diagnostic(code(livtet_cli::verify_report_missing_plugin_id))]
    VerifyReportMissingPluginId,

    #[error("verify report missing version")]
    #[diagnostic(code(livtet_cli::verify_report_missing_version))]
    VerifyReportMissingVersion,

    #[error("--version is required when --repo is specified")]
    #[diagnostic(code(livtet_cli::missing_version))]
    MissingVersion,

    #[error("repository {repo_name:?} not found in repositories.toml")]
    #[diagnostic(code(livtet_cli::repository_not_found))]
    RepositoryNotFound { repo_name: String },

    #[error("plugin {plugin_id:?} v{version} not found in repo {repo_name:?}")]
    #[diagnostic(code(livtet_cli::plugin_version_not_found))]
    PluginVersionNotFound {
        plugin_id: String,
        version: String,
        repo_name: String,
    },

    #[error("failed to read cached index for repo {repo_name:?}")]
    #[diagnostic(code(livtet_cli::index_read_failed))]
    IndexReadFailed {
        repo_name: String,
        #[source]
        source: std::io::Error,
    },

    #[error("failed to parse cached index for repo {repo_name:?}: {message}")]
    #[diagnostic(code(livtet_cli::index_parse_failed))]
    IndexParseFailed { repo_name: String, message: String },

    #[error("failed to load HMAC key")]
    #[diagnostic(code(livtet_cli::hmac_load))]
    HmacLoad {
        #[source]
        source: Box<CliError>,
    },

    #[error("install failed: {message}")]
    #[diagnostic(code(livtet_cli::install_failed))]
    InstallFailed { message: String },

    #[error("download failed: {message}")]
    #[diagnostic(code(livtet_cli::download_failed))]
    DownloadFailed { message: String },

    #[error("download failed: HTTP {status} for {url}")]
    #[diagnostic(code(livtet_cli::download_http_error))]
    DownloadHttpError { status: u16, url: String },

    #[error("non-utf8 path: {path:?}")]
    #[diagnostic(code(livtet_cli::non_utf8_path))]
    NonUtf8Path { path: std::path::PathBuf },

    #[error("failed to build tokio runtime: {message}")]
    #[diagnostic(code(livtet_cli::tokio_runtime_build))]
    TokioRuntimeBuild { message: String },

    #[error("plugin {id} v{version} is not installed at {path}")]
    #[diagnostic(code(livtet_cli::plugin_not_installed))]
    PluginNotInstalled {
        id: String,
        version: String,
        path: std::path::PathBuf,
    },

    #[error("failed to record install: {message}")]
    #[diagnostic(code(livtet_cli::install_record_failed))]
    InstallRecordFailed { message: String },

    #[error("failed to load repositories.toml: {message}")]
    #[diagnostic(code(livtet_cli::repositories_toml_load_failed))]
    RepositoriesTomlLoadFailed { message: String },

    #[error("pack failed: {message}")]
    #[diagnostic(code(livtet_cli::pack_failed))]
    PackFailed { message: String },

    #[error("operation failed: {message}")]
    #[diagnostic(code(livtet_cli::operation))]
    Operation { message: String },

    #[error("interactive prompt failed: {message}")]
    #[diagnostic(code(livtet_cli::interactive_aborted))]
    InteractiveAborted { message: String },
}

pub type Result<T> = miette::Result<T, CliError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_passphrase_display() {
        let msg = CliError::EmptyPassphrase.to_string();
        assert!(msg.contains("Passphrase is empty"));
    }

    #[test]
    fn missing_version_display() {
        let msg = CliError::MissingVersion.to_string();
        assert!(msg.contains("--version"));
    }

    #[test]
    fn signing_key_changed_has_help() {
        let msg = CliError::SigningKeyChanged.to_string();
        assert!(msg.contains("signing key changed"));
        let help = format!("{:?}", CliError::SigningKeyChanged);
        assert!(help.contains("SigningKeyChanged"));
    }

    #[test]
    fn verify_report_missing_plugin_id_display() {
        let msg = CliError::VerifyReportMissingPluginId.to_string();
        assert!(msg.contains("plugin_id"));
    }

    #[test]
    fn verify_report_missing_version_display() {
        let msg = CliError::VerifyReportMissingVersion.to_string();
        assert!(msg.contains("version"));
    }

    #[test]
    fn io_error_converts_via_from() {
        let io_err = std::io::Error::new(std::io::ErrorKind::NotFound, "missing");
        let converted: CliError = io_err.into();
        match converted {
            CliError::Io(_) => {}
            other => panic!("expected CliError::Io, got {other:?}"),
        }
    }
}
