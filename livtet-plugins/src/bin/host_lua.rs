#[tokio::main]
async fn main() -> std::process::ExitCode {
    livtet_plugins::host::run(std::env::args().collect()).await
}
