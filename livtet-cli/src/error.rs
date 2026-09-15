use miette::Diagnostic;
use thiserror::Error;

#[derive(Error, Debug, Diagnostic)]
#[allow(clippy::disallowed_types)]
pub enum CliError {
    #[error("I/O error: {0}")]
    #[diagnostic(code(livtet_cli::io))]
    Io(#[from] std::io::Error),

    #[error("operation failed: {message}")]
    #[diagnostic(code(livtet_cli::operation))]
    Operation { message: String },
}

pub type Result<T> = miette::Result<T, CliError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn operation_display() {
        let msg = CliError::Operation {
            message: "boom".to_string(),
        }
        .to_string();
        assert!(msg.contains("boom"));
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
