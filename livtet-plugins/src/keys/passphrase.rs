use std::io::IsTerminal;

use camino::Utf8PathBuf;

use crate::archive::error::ArchiveError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PassphraseSource {
    NoPassphrase,
    EnvVar,
    Interactive,
}

pub struct ResolvedPassphrase(pub String, pub PassphraseSource);

pub fn resolve_passphrase(
    no_passphrase: bool,
    env_var_name: Option<&str>,
    is_tty: bool,
) -> Result<ResolvedPassphrase, ArchiveError> {
    if no_passphrase {
        return Ok(ResolvedPassphrase(
            String::new(),
            PassphraseSource::NoPassphrase,
        ));
    }
    if let Some(name) = env_var_name
        && let Ok(value) = std::env::var(name)
    {
        return Ok(ResolvedPassphrase(value, PassphraseSource::EnvVar));
    }
    if is_tty && std::io::stdin().is_terminal() && std::io::stdout().is_terminal() {
        let pw = rpassword::prompt_password("Passphrase: ")
            .map_err(|e| ArchiveError::Key(format!("passphrase prompt failed: {e}")))?;
        return Ok(ResolvedPassphrase(pw, PassphraseSource::Interactive));
    }
    Err(ArchiveError::PassphraseRequired {
        key_path: Utf8PathBuf::from("(unknown)"),
    })
}
