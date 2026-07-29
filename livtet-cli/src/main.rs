use livtet_cli::run;

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();
    if let Err(report) = run() {
        // Print the report via its `Display` impl rather than the
        // default `Termination` `Debug` format. The default would
        // surface the concrete `thiserror` enum's variant name and
        // field list, e.g.
        // `Error: PluginNotInstalled { id: "x", version: "0.1.0", ... }`,
        // which is unreadable for end users. Routing through Display
        // walks the `source()` chain and emits the human-readable
        // message + any `#[help]` annotations.
        eprintln!("Error: {report}");
        std::process::exit(1);
    }
}
