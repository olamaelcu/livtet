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
    /// Name advertised to nearby peers; when `None` (or blank) the daemon
    /// generates one from the device id.
    pub device_name: Option<String>,
    /// Where received book files are written; required before pulling offers.
    pub books_dir: Option<Utf8PathBuf>,
    /// Where received covers are written; required before pulling offers.
    pub covers_dir: Option<Utf8PathBuf>,
    /// How often the pull loop reads every outbound peer.
    ///
    /// The peer cannot push, so a shorter interval is the only way a book
    /// arrives sooner; it costs one dial and one query per peer per tick.
    pub pull_interval_secs: u64,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            db_path: Utf8PathBuf::from("livtet-sync.db"),
            host: "127.0.0.1".to_string(),
            port: 0,
            device_id: None,
            device_name: None,
            books_dir: None,
            covers_dir: None,
            pull_interval_secs: 60,
        }
    }
}

impl ServerConfig {
    /// Parse `--db <path> --host <h> --lan --port <n> --device-id <id>
    /// --device-name <name> --books-dir <path> --covers-dir <path>` from an
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
                "--lan" => {
                    // Opt in to a LAN-reachable bind. Every `/sync/*` data
                    // route requires a session token, and pairing still needs
                    // approval on this device.
                    config.host = "0.0.0.0".to_string();
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
                "--device-name" => {
                    let value = args.next().ok_or_else(|| {
                        ServerError::Config("--device-name requires a value".to_string())
                    })?;
                    config.device_name = Some(value);
                }
                "--books-dir" => {
                    let value = args.next().ok_or_else(|| {
                        ServerError::Config("--books-dir requires a path".to_string())
                    })?;
                    config.books_dir = Some(Utf8PathBuf::from(value));
                }
                "--pull-interval-secs" => {
                    let value = args.next().ok_or_else(|| {
                        ServerError::Config("--pull-interval-secs requires a value".to_string())
                    })?;
                    config.pull_interval_secs = value.parse().map_err(|_| {
                        ServerError::Config(format!("invalid pull interval: {value}"))
                    })?;
                }
                "--covers-dir" => {
                    let value = args.next().ok_or_else(|| {
                        ServerError::Config("--covers-dir requires a path".to_string())
                    })?;
                    config.covers_dir = Some(Utf8PathBuf::from(value));
                }
                other => {
                    return Err(ServerError::Config(format!("unknown argument: {other}")));
                }
            }
        }

        Ok(config)
    }
}
