use std::{
    io::{self, Write},
    process::ExitCode,
    sync::{Arc, Mutex},
};

use camino::Utf8PathBuf;
use clap::Parser;
use livtet_core::paths;
use livtet_plugins::{
    host_lua::LuaHost,
    ipc_host::IpcHost,
    protocol::{HostToMain, MainToHost, MainToHostCallback},
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    sync::mpsc,
};
use tracing_appender::{
    non_blocking::WorkerGuard,
    rolling::{RollingFileAppender, Rotation},
};
use tracing_subscriber::{EnvFilter, fmt, layer::SubscriberExt, util::SubscriberInitExt};

#[derive(Parser)]
#[command(name = "livtet-plugins-host-lua")]
#[command(about = "Plugin host process for Lua plugins")]
struct Cli {
    #[arg(long, env = "LIVTET_LOG_DIR", value_name = "LOG_DIR")]
    log_dir: Option<Utf8PathBuf>,
}

static LOG_GUARD: std::sync::Mutex<Option<WorkerGuard>> = std::sync::Mutex::new(None);

#[derive(Clone)]
struct MessageTransport;

impl MessageTransport {
    fn new() -> Self {
        Self
    }

    async fn read_msg<T: serde::de::DeserializeOwned>(&self) -> io::Result<T> {
        let mut stdin = tokio::io::stdin();
        let mut len_buf = [0u8; 4];
        stdin.read_exact(&mut len_buf).await?;
        let len = u32::from_le_bytes(len_buf) as usize;
        let mut payload = vec![0u8; len];
        stdin.read_exact(&mut payload).await?;
        rmp_serde::from_slice(&payload).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
    }

    async fn write_msg<T: serde::Serialize>(&self, msg: &T) -> io::Result<()> {
        let mut stdout = tokio::io::stdout();
        let payload = rmp_serde::to_vec_named(msg)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        let len = (payload.len() as u32).to_le_bytes();
        stdout.write_all(&len).await?;
        stdout.write_all(&payload).await?;
        stdout.flush().await?;
        Ok(())
    }
}

#[derive(Clone, serde::Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum MainMessage {
    LoadPlugin {
        plugin_id: String,
        manifest: serde_json::Value,
        source: String,
        #[serde(default)]
        data_dir: Option<camino::Utf8PathBuf>,
        #[serde(default)]
        settings: Option<std::collections::HashMap<String, String>>,
        /// LuaRocks rock names this plugin depends on (mirrors
        /// `PluginMeta.rocks`). The host logs these for now;
        /// a follow-up will use them to pre-register expected
        /// rocks with the Lua loader. `#[serde(default)]` keeps
        /// older senders compatible.
        #[serde(default)]
        rocks: Vec<String>,
    },
    UnloadPlugin {
        plugin_id: String,
    },
    Call {
        id: String,
        plugin_id: String,
        capability: String,
        args: Vec<serde_json::Value>,
    },
    Shutdown,
    HttpResponse {
        id: String,
        status: u16,
        #[serde(default)]
        body: Option<String>,
        #[serde(default)]
        headers: Vec<(String, String)>,
    },
    GetSecretResult {
        id: String,
        #[serde(default)]
        value: Option<String>,
        #[serde(default)]
        error: Option<String>,
    },
    #[allow(dead_code)]
    SettingResult {
        id: String,
        #[serde(default)]
        ok: bool,
        #[serde(default)]
        error: Option<String>,
    },
    SetSecretResult {
        id: String,
        #[serde(default)]
        error: Option<String>,
    },
    SecretResult {
        id: String,
        #[serde(default)]
        value: Option<String>,
        #[serde(default)]
        error: Option<String>,
    },
    ReadFileResult {
        id: String,
        #[serde(default)]
        content: Option<String>,
        #[serde(default)]
        error: Option<String>,
    },
    SqliteResult {
        id: String,
        #[serde(default)]
        columns: Vec<String>,
        #[serde(default)]
        rows: Vec<Vec<serde_json::Value>>,
        #[serde(default)]
        error: Option<String>,
    },
    AssetResult {
        id: String,
        #[serde(default)]
        content: Option<String>,
        #[serde(default)]
        error: Option<String>,
    },
    ResolveIdentifierResult {
        id: String,
        #[serde(default)]
        edition_id: Option<String>,
        #[serde(default)]
        error: Option<String>,
    },
    ResolveIdentifiersResult {
        id: String,
        #[serde(default)]
        edition_ids: Vec<Option<String>>,
        #[serde(default)]
        error: Option<String>,
    },
    EditionInfoResult {
        id: String,
        #[serde(default)]
        info: Option<serde_json::Value>,
        #[serde(default)]
        error: Option<String>,
    },
    EditionIdentifiersResult {
        id: String,
        #[serde(default)]
        urns: Vec<String>,
        #[serde(default)]
        error: Option<String>,
    },
    FetchProgressResult {
        id: String,
        #[serde(default)]
        progress: Option<livtet_plugins::progress_entry::ProgressEntry>,
        #[serde(default)]
        error: Option<String>,
    },
    UpsertProgressResult {
        id: String,
        #[serde(default)]
        edition_id: Option<String>,
        #[serde(default)]
        format_id: Option<String>,
        #[serde(default)]
        ok: bool,
        #[serde(default)]
        error: Option<String>,
    },
}

impl MainMessage {
    fn into_request(self) -> Option<MainToHost> {
        match self {
            MainMessage::LoadPlugin {
                plugin_id,
                manifest,
                source,
                data_dir,
                settings,
                rocks,
            } => Some(MainToHost::LoadPlugin {
                plugin_id,
                manifest,
                source,
                data_dir,
                settings,
                rocks,
            }),
            MainMessage::UnloadPlugin { plugin_id } => Some(MainToHost::UnloadPlugin { plugin_id }),
            MainMessage::Call {
                id,
                plugin_id,
                capability,
                args,
            } => Some(MainToHost::Call {
                id,
                plugin_id,
                capability,
                args,
            }),
            MainMessage::Shutdown => Some(MainToHost::Shutdown),
            _ => None,
        }
    }

    fn into_callback(self) -> Option<MainToHostCallback> {
        match self {
            MainMessage::HttpResponse {
                id,
                status,
                body,
                headers,
            } => Some(MainToHostCallback::HttpResponse {
                id,
                status,
                body,
                headers,
            }),
            MainMessage::GetSecretResult { id, value, error } => {
                Some(MainToHostCallback::SecretResult { id, value, error })
            }
            MainMessage::SetSecretResult { id, error } => Some(MainToHostCallback::SecretResult {
                id,
                value: None,
                error,
            }),
            MainMessage::SecretResult { id, value, error } => {
                Some(MainToHostCallback::SecretResult { id, value, error })
            }
            MainMessage::ReadFileResult { id, content, error } => {
                Some(MainToHostCallback::ReadFileResult { id, content, error })
            }
            MainMessage::SqliteResult {
                id,
                columns,
                rows,
                error,
            } => Some(MainToHostCallback::SqliteResult {
                id,
                columns,
                rows,
                error,
            }),
            MainMessage::AssetResult { id, content, error } => {
                Some(MainToHostCallback::AssetResult { id, content, error })
            }
            MainMessage::ResolveIdentifierResult {
                id,
                edition_id,
                error,
            } => Some(MainToHostCallback::ResolveIdentifierResult {
                id,
                edition_id,
                error,
            }),
            MainMessage::ResolveIdentifiersResult {
                id,
                edition_ids,
                error,
            } => Some(MainToHostCallback::ResolveIdentifiersResult {
                id,
                edition_ids,
                error,
            }),
            MainMessage::EditionInfoResult { id, info, error } => {
                Some(MainToHostCallback::EditionInfoResult { id, info, error })
            }
            MainMessage::EditionIdentifiersResult { id, urns, error } => {
                Some(MainToHostCallback::EditionIdentifiersResult { id, urns, error })
            }
            MainMessage::FetchProgressResult {
                id,
                progress,
                error,
            } => Some(MainToHostCallback::FetchProgressResult {
                id,
                progress,
                error,
            }),
            MainMessage::UpsertProgressResult {
                id,
                edition_id,
                format_id,
                ok,
                error,
            } => Some(MainToHostCallback::UpsertProgressResult {
                id,
                edition_id,
                format_id,
                ok,
                error,
            }),
            _ => None,
        }
    }
}

const ENV_LOG_DIR: &str = "LIVTET_LOG_DIR";

fn resolve_log_dir(explicit: Option<&camino::Utf8Path>) -> Option<Utf8PathBuf> {
    if let Some(p) = explicit {
        return Some(p.to_path_buf());
    }
    if let Ok(p) = std::env::var(ENV_LOG_DIR) {
        let p = Utf8PathBuf::from(p);
        if !p.as_str().is_empty() {
            return Some(p);
        }
    }
    Some(paths::logs_dir())
}

fn init_logging(explicit: Option<&camino::Utf8Path>) -> io::Result<()> {
    let env_filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("debug"));

    let stderr_layer = fmt::layer()
        .with_writer(io::stderr)
        .with_ansi(false)
        .with_target(true)
        .with_level(true)
        .with_thread_ids(false)
        .with_file(true)
        .with_line_number(true);

    let registry = tracing_subscriber::registry()
        .with(env_filter)
        .with(stderr_layer);

    let Some(dir) = resolve_log_dir(explicit) else {
        eprintln!(
            "plugin host logging: cannot determine a log directory; \
             refusing to start (set {ENV_LOG_DIR} or pass --log-dir)"
        );
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            "no log directory resolvable",
        ));
    };

    if let Err(e) = fs_err::create_dir_all(dir.as_std_path()) {
        eprintln!("plugin host logging: cannot create log dir {dir}: {e}");
        return Err(e);
    }
    let probe = dir.join(".write-probe");
    if let Err(e) = fs_err::write(probe.as_std_path(), b"") {
        eprintln!("plugin host logging: log dir {dir} is not writable: {e}");
        return Err(e);
    }
    let _ = fs_err::remove_file(probe.as_std_path());

    let file_appender =
        RollingFileAppender::new(Rotation::DAILY, dir.as_std_path(), "plugin-host.log");
    let (file_writer, guard) = tracing_appender::non_blocking(file_appender);

    *LOG_GUARD.lock().unwrap() = Some(guard);

    let file_layer = fmt::layer()
        .with_writer(file_writer)
        .with_ansi(false)
        .with_target(true)
        .with_level(true)
        .with_thread_ids(false)
        .with_file(true)
        .with_line_number(true);

    registry.with(file_layer).try_init().ok();

    tracing::info!(
        log_dir = %dir,
        "plugin host logging initialized"
    );

    Ok(())
}

fn flush_logs() {
    tracing::info!("flushing logs and shutting down logging");
    if let Ok(mut guard) = LOG_GUARD.lock() {
        guard.take();
    }
}

struct StdoutWriter;

impl Write for StdoutWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        io::stdout().write(buf)
    }
    fn flush(&mut self) -> io::Result<()> {
        io::stdout().flush()
    }
}

#[tokio::main]
async fn main() -> ExitCode {
    let cli = Cli::parse();

    if let Err(e) = init_logging(cli.log_dir.as_deref()) {
        eprintln!("failed to initialize logging: {}", e);
        return ExitCode::from(1);
    }

    tracing::info!("plugin host starting up");

    std::panic::set_hook(Box::new(|info| {
        tracing::error!("host panicked: {}", info);
    }));

    let stdout_arc: Arc<Mutex<Box<dyn Write + Send>>> =
        Arc::new(Mutex::new(Box::new(StdoutWriter)));
    let writer_for_main: Arc<Mutex<Box<dyn Write + Send>>> = Arc::clone(&stdout_arc);
    let router: livtet_plugins::ipc_host::CallbackRouter =
        Arc::new(Mutex::new(std::collections::HashMap::new()));

    let host_impl = Arc::new(IpcHost::new(
        Arc::clone(&writer_for_main),
        Arc::clone(&router),
    ));

    let mut host = match LuaHost::new(host_impl) {
        Ok(h) => h,
        Err(e) => {
            tracing::error!("failed to create lua host: {}", e);
            return ExitCode::from(1);
        }
    };

    host.set_ipc_writer(Arc::clone(&stdout_arc));

    let (request_tx, mut request_rx) = mpsc::channel::<MainToHost>(32);
    let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();
    let request_tx_for_reader = request_tx.clone();
    let router_for_reader = Arc::clone(&router);

    let transport = MessageTransport::new();
    let transport_for_reader = transport.clone();

    let _reader_handle = tokio::spawn(async move {
        let mut shutdown_rx = shutdown_rx;
        loop {
            let msg: MainMessage = tokio::select! {
                result = transport_for_reader.read_msg() => match result {
                    Ok(m) => m,
                    Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => {
                        tracing::warn!("host reader: stdin closed (EOF); awaiting explicit shutdown");
                        break;
                    }
                    Err(e) => {
                        tracing::error!("host reader: read failed: {}", e);
                        break;
                    }
                },
                _ = &mut shutdown_rx => {
                    tracing::info!("host reader: shutdown signal received");
                    break;
                }
            };
            #[allow(clippy::match_single_binding)]
            match msg {
                msg => {
                    if let Some(cb) = msg.clone().into_callback() {
                        let id = callback_id(&cb);
                        let map = router_for_reader.lock().expect("router mutex poisoned");
                        if let Some(tx) = map.get(&id) {
                            let _ = tx.send(cb);
                        }
                    } else if let Some(req) = msg.into_request()
                        && request_tx_for_reader.send(req).await.is_err()
                    {
                        break;
                    }
                }
            }
        }
    });

    let ready = HostToMain::Ready {
        runtime: "lua".to_string(),
    };
    if let Err(e) = transport.write_msg(&ready).await {
        tracing::error!("failed to write ready: {}", e);
        return ExitCode::from(1);
    }

    let _keep_request_tx_alive = request_tx;

    while let Some(req) = request_rx.recv().await {
        if matches!(req, MainToHost::Shutdown) {
            tracing::info!("received shutdown signal");
            break;
        }
        if let MainToHost::LoadPlugin {
            plugin_id, rocks, ..
        } = &req
        {
            tracing::info!(
                plugin = %plugin_id,
                rocks = ?rocks,
                "LoadPlugin: rocks declared by manifest"
            );
        }
        if let Some(response) = host.handle_message(req)
            && let Err(e) = transport.write_msg(&response).await
        {
            tracing::error!("failed to write response: {}", e);
            return ExitCode::from(1);
        }
    }

    eprintln!("[host-debug] main loop exited, about to flush_logs");
    flush_logs();
    eprintln!("[host-debug] flush_logs returned, sending shutdown_tx");

    let _ = shutdown_tx.send(());
    eprintln!("[host-debug] shutdown_tx sent, dropping request_tx");

    drop(_keep_request_tx_alive);
    eprintln!("[host-debug] request_tx dropped, about to return from main");
    ExitCode::from(0)
}

fn callback_id(cb: &MainToHostCallback) -> String {
    match cb {
        MainToHostCallback::HttpResponse { id, .. }
        | MainToHostCallback::SecretResult { id, .. }
        | MainToHostCallback::SettingResult { id, .. }
        | MainToHostCallback::ReadFileResult { id, .. }
        | MainToHostCallback::SqliteResult { id, .. }
        | MainToHostCallback::AssetResult { id, .. }
        | MainToHostCallback::ResolveIdentifierResult { id, .. }
        | MainToHostCallback::ResolveIdentifiersResult { id, .. }
        | MainToHostCallback::EditionInfoResult { id, .. }
        | MainToHostCallback::EditionIdentifiersResult { id, .. }
        | MainToHostCallback::FetchProgressResult { id, .. }
        | MainToHostCallback::UpsertProgressResult { id, .. }
        | MainToHostCallback::StoreEmbeddingResult { id, .. }
        | MainToHostCallback::GetEmbeddingResult { id, .. }
        | MainToHostCallback::FindSimilarEditionsResult { id, .. }
        | MainToHostCallback::OAuthRedeemResult { id, .. }
        | MainToHostCallback::OAuthTokenResult { id, .. }
        | MainToHostCallback::OAuthRevokeResult { id, .. }
        | MainToHostCallback::OAuthAuthorizeResult { id, .. } => id.clone(),
    }
}
