use crate::{CliError, Result};

pub fn success(msg: &str) {
    println!("{msg}");
}

pub fn info(msg: &str) {
    eprintln!("info: {msg}");
}

pub fn warn(msg: &str) {
    eprintln!("warning: {msg}");
}

pub fn error(msg: &str) {
    eprintln!("error: {msg}");
}

/// Non-interactive confirmation prompt. Reads a single line from stdin
/// and accepts only a case-insensitive `y`. Returns `Ok(false)` on
/// EOF or any other input — callers that want the user to confirm a
/// destructive action treat a non-`y` response as "no".
pub fn prompt_confirm(question: &str) -> Result<bool> {
    use std::io::Write;
    eprint!("{question} [y/N] ");
    std::io::stderr().flush()?;
    let mut line = String::new();
    std::io::stdin().read_line(&mut line)?;
    Ok(line.trim().eq_ignore_ascii_case("y"))
}

/// Interactive confirmation prompt backed by `inquire::Confirm`. Use
/// this for destructive operations (uninstall, unpublish) when the
/// caller has opted into `--interactive` mode. The default answer is
/// `false` so a stray Enter cannot accidentally confirm a destructive
/// action.
pub fn prompt_confirm_interactive(question: &str, default: bool) -> Result<bool> {
    inquire::Confirm::new(question)
        .with_default(default)
        .prompt()
        .map_err(|e| CliError::InteractiveAborted {
            message: format!("confirmation prompt failed: {e}"),
        })
}

/// Build a fresh `indicatif` progress bar. Centralised here so the
/// style (ticker character, elapsed-time template) stays consistent
/// across `pack`, `install`, and any other long-running CLI commands.
///
/// Returns a bar sized to `len` with `msg` as the prefix and the
/// default bytes/elapsed template. Callers are expected to wrap work
/// in a `set_position`/`finish` block, or just `finish_and_clear` if
/// they want the bar to disappear on completion.
pub fn progress_bar(len: u64, msg: &str) -> indicatif::ProgressBar {
    let pb = indicatif::ProgressBar::new(len);
    pb.set_style(
        indicatif::ProgressStyle::default_bar()
            .template("{msg} [{bar:30.cyan/blue}] {pos}/{len} {eta_precise}")
            .expect("static template")
            .progress_chars("=>-"),
    );
    pb.set_message(msg.to_string());
    pb
}

/// Wrap a synchronous closure in an `indicatif` progress bar sized to
/// `len`. The closure receives the bar and is expected to call
/// `inc(delta)` as it makes progress. The bar is finished and cleared
/// on the way out, so the user sees a clean terminal regardless of
/// the path taken through the closure.
pub fn with_progress<F, T>(len: u64, msg: &str, mut step: F) -> Result<T>
where
    F: FnMut(&indicatif::ProgressBar) -> Result<T>,
{
    let pb = progress_bar(len, msg);
    let result = step(&pb);
    pb.finish_and_clear();
    result
}
