//! Configuration for the sync server daemon.

use camino::Utf8PathBuf;

use crate::error::ServerError;

/// Runtime configuration for [`crate::run`].
#[derive(Debug, Clone)]
pub struct ServerConfig {
    /// Path to the SQLite database file (a bare path or `sqlite:` URL).
    pub db_path: Utf8PathBuf,
    /// Host/interface to bind the sync HTTP server on.
    pub host: String,
    /// Port to bind; `0` asks the OS for an ephemeral port.
    pub port: u16,
    /// Optional pre-set device id; when `None` the daemon reads
    /// `client_settings.device_id` and generates a ULID if absent.
    pub device_id: Option<String>,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            db_path: Utf8PathBuf::from("livtet-sync.db"),
            host: "127.0.0.1".to_string(),
            port: 0,
            device_id: None,
        }
    }
}

impl ServerConfig {
    /// Parse `--db <path> --host <h> --port <n> --device-id <id>` from an
    /// argument iterator (typically `std::env::args().skip(1)`).
    pub fn from_args(args: impl Iterator<Item = String>) -> Result<Self, ServerError> {
        let mut config = Self::default();
        let mut args = args.peekable();

        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--db" => {
                    let value = args.next().ok_or_else(|| {
                        ServerError::Config("--db requires a path argument".to_string())
                    })?;
                    config.db_path = Utf8PathBuf::from(value);
                }
                "--host" => {
                    config.host = args.next().ok_or_else(|| {
                        ServerError::Config("--host requires a value".to_string())
                    })?;
                }
                "--port" => {
                    let value = args.next().ok_or_else(|| {
                        ServerError::Config("--port requires a value".to_string())
                    })?;
                    config.port = value.parse::<u16>().map_err(|_| {
                        ServerError::Config(format!("invalid --port value: {value}"))
                    })?;
                }
                "--device-id" => {
                    let value = args.next().ok_or_else(|| {
                        ServerError::Config("--device-id requires a value".to_string())
                    })?;
                    config.device_id = Some(value);
                }
                other => {
                    return Err(ServerError::Config(format!("unknown argument: {other}")));
                }
            }
        }

        Ok(config)
    }
}
