//! Binary entry point for the Livtet sync server daemon.

use livtet_sync_server::ServerConfig;

const USAGE: &str = "\
livtet-sync-server — Livtet sync daemon (iroh streams + JSON-RPC over stdio)

USAGE:
    livtet-sync-server [OPTIONS]

OPTIONS:
    --db <path>         SQLite database path (default: livtet-sync.db)
    --port <port>       UDP port for the iroh endpoint; 0 picks an ephemeral port (default: 0)
    --device-id <id>    Override the persisted device id (ULID)
    --device-name <n>   Name advertised to nearby peers
    --books-dir <path>  Where received book files are written
    --covers-dir <path> Where received covers are written
    --pull-interval-secs <n>  How often to pull from outbound peers (default: 60)
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
