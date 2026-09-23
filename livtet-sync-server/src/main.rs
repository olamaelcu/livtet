//! Binary entry point for the Livtet sync server daemon.

use livtet_sync_server::ServerConfig;

const USAGE: &str = "\
livtet-sync-server — Livtet sync daemon (HTTP server + JSON-RPC over stdio)

USAGE:
    livtet-sync-server [OPTIONS]

OPTIONS:
    --db <path>         SQLite database path (default: livtet-sync.db)
    --host <host>       Host to bind the sync HTTP server (default: 127.0.0.1)
    --port <port>       Port to bind; 0 picks an ephemeral port (default: 0)
    --device-id <id>    Override the persisted device id (ULID)
    -h, --help          Print this help
";

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();

    if args.iter().any(|arg| arg == "--help" || arg == "-h") {
        println!("{}", USAGE);
        return;
    }

    // Structured logs go to stderr; stdout is reserved for the NDJSON RPC
    // protocol. `try_init` tolerates an already-installed subscriber.
    let _ = tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .try_init();

    let config = match ServerConfig::from_args(args.into_iter()) {
        Ok(config) => config,
        Err(error) => {
            eprintln!("error: {error}\n\n{}", USAGE);
            std::process::exit(2);
        }
    };

    if let Err(error) = livtet_sync_server::run(config) {
        eprintln!("fatal: {error}");
        std::process::exit(1);
    }
}
