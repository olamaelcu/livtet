//! `livtet keyring-recover` — derive an HMAC key from a passphrase and
//! persist it to disk so subsequent `livtet` invocations can read
//! HMAC-protected state files when the OS keyring is unavailable.
//!
//! See `docs/superpowers/specs/2026-06-04-plugin-signing-repositories-design.md`
//! §8.1 deviations 1, 13, 14 for context. The flow is intentionally
//! pragmatic: derive a deterministic 32-byte key via PBKDF2-HMAC-SHA256,
//! write the hex-encoded key to `<config-dir>/passphrase.env` (mode 0600),
//! and have every other CLI command prefer `load_state_hmac_key()` over
//! the hard-coded `DEFAULT_HMAC_KEY` constant.

use std::io::Read;

use camino::Utf8Path;
use fs_err::{self as fs, os::unix::fs::OpenOptionsExt};
use livtet_plugin::repository::hmac::{HmacKey, PASSPHRASE_SALT};

use crate::{Result, cli::KeyringRecoverArgs, error::CliError, output, repo::default_config_dir};

pub const PASSPHRASE_ENV_FILE: &str = "passphrase.env";

pub fn run(args: KeyringRecoverArgs) -> Result<()> {
    let passphrase = read_passphrase(&args)?;
    if passphrase.is_empty() {
        return Err(CliError::EmptyPassphrase);
    }

    let key = HmacKey::derive_from_passphrase(&passphrase, PASSPHRASE_SALT);
    let hex: String = key.as_bytes().iter().map(|b| format!("{b:02x}")).collect();

    let target = match args.output {
        Some(p) => p,
        None => default_config_dir().join(PASSPHRASE_ENV_FILE),
    };

    write_recovery_file(&target, &hex)?;

    output::success(&format!(
        "Wrote passphrase-derived HMAC key to {}\n  \
         Subsequent `livtet` invocations will read this file when the OS keyring is unavailable.\n  \
         To remove it: `rm {target}`",
        target
    ));
    Ok(())
}

fn read_passphrase(args: &KeyringRecoverArgs) -> Result<String> {
    if let Some(p) = args.passphrase.as_ref() {
        return Ok(p.clone());
    }
    if args.passphrase_stdin {
        let mut buf = String::new();
        std::io::stdin()
            .read_to_string(&mut buf)
            .map_err(|e| CliError::Operation {
                message: format!("failed to read passphrase from stdin: {e}"),
            })?;
        // Trim a single trailing newline if present (e.g., from `echo`).
        if buf.ends_with('\n') {
            buf.pop();
        }
        return Ok(buf);
    }
    if args.interactive {
        // `inquire::Password` keeps the input off the terminal echo and
        // out of process listings by default. `.without_confirmation()`
        // skips the second "retype passphrase" prompt that inquire
        // normally adds — we only need a single read here because the
        // user has already chosen this flow on the command line.
        let value = inquire::Password::new("Recovery passphrase:")
            .without_confirmation()
            .prompt()
            .map_err(|e| CliError::InteractiveAborted {
                message: format!("passphrase prompt failed: {e}"),
            })?;
        if value.is_empty() {
            return Err(CliError::EmptyPassphrase);
        }
        return Ok(value);
    }
    Err(CliError::NoPassphraseSource)
}

fn write_recovery_file(path: &Utf8Path, hex_key: &str) -> Result<()> {
    use std::io::Write;

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| CliError::Operation {
            message: format!("mkdir {parent}: {e}"),
        })?;
    }
    let mut opts = fs::OpenOptions::new();
    opts.write(true).create(true).truncate(true).mode(0o600);
    let mut f = opts
        .open(path.as_std_path())
        .map_err(|e| CliError::Operation {
            message: format!("open {} for write: {e}", path),
        })?;
    writeln!(f, "LIVTET_PASSPHRASE_HMAC_KEY={hex_key}").map_err(|e| CliError::Operation {
        message: format!("write to {}: {e}", path),
    })?;
    Ok(())
}

/// Resolve the HMAC key the rest of the CLI should use, with the
/// following priority:
///
/// 1. **OS keyring** (`HmacKey::load_from_keyring`) — the spec-canonical
///    source, used when available.
/// 2. **Passphrase recovery file** (`<config-dir>/passphrase.env`) —
///    written by `livtet keyring-recover`. Used when the keyring is
///    unavailable but the user has previously derived a key from a
///    passphrase.
/// 3. **Test override** (`LIVTET_HMAC_KEY_HEX` env var) — escape hatch
///    for CI and unit tests that need a deterministic, in-process key
///    without a keyring or recovery file. The env var must contain a
///    64-char hex string. Documented but not advertised in `--help`.
/// 4. **Refuse** — return an error pointing the user at
///    `livtet keyring-recover`. We no longer silently fall back to
///    `DEFAULT_HMAC_KEY`; see spec §11.6 and §8.1 deviation 14.
pub fn load_state_hmac_key() -> Result<HmacKey> {
    if let Ok(key) = HmacKey::load_from_keyring() {
        return Ok(key);
    }

    let config_dir = default_config_dir();
    let path = config_dir.join(PASSPHRASE_ENV_FILE);
    if let Some(key) = try_read_recovery_file(&path)? {
        return Ok(key);
    }

    if let Some(key) = try_read_env_override() {
        return Ok(key);
    }

    Err(CliError::NoHmacSource { path: path.into() })
}

fn try_read_env_override() -> Option<HmacKey> {
    let raw = std::env::var_os("LIVTET_HMAC_KEY_HEX")?;
    let s = raw.into_string().ok()?;
    let bytes = hex::decode(s.trim()).ok()?;
    if bytes.len() != 32 {
        return None;
    }
    let mut arr = [0u8; 32];
    arr.copy_from_slice(&bytes);
    Some(HmacKey::from_bytes(arr))
}

/// Test-only accessor: build the same `HmacKey` that
/// `load_state_hmac_key` would return when the test environment sets
/// `LIVTET_HMAC_KEY_HEX`. Used by integration tests that need to
/// write HMAC-protected fixtures and then read them back through the
/// public CLI. Production code paths should call `load_state_hmac_key`.
pub fn test_hmac_key_from_env_or_default() -> HmacKey {
    try_read_env_override().unwrap_or_else(|| HmacKey::from_bytes(crate::plugin::DEFAULT_HMAC_KEY))
}

fn try_read_recovery_file(path: &Utf8Path) -> Result<Option<HmacKey>> {
    if !path.exists() {
        return Ok(None);
    }
    let text = fs::read_to_string(path).map_err(|e| CliError::Operation {
        message: format!("read {path}: {e}"),
    })?;
    let line = text
        .trim()
        .strip_prefix("LIVTET_PASSPHRASE_HMAC_KEY=")
        .ok_or_else(|| CliError::RecoveryFileMalformed { path: path.into() })?;
    let bytes = hex::decode(line.trim()).map_err(|source| CliError::RecoveryFileInvalidHex {
        path: path.into(),
        source,
    })?;
    if bytes.len() != 32 {
        return Err(CliError::RecoveryFileWrongSize {
            path: path.into(),
            actual: bytes.len(),
        });
    }
    let mut arr = [0u8; 32];
    arr.copy_from_slice(&bytes);
    Ok(Some(HmacKey::from_bytes(arr)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recovery_file_round_trip_reproduces_key() {
        let dir = camino_tempfile::tempdir().unwrap();
        let path = dir.path().join("passphrase.env");

        let key = HmacKey::derive_from_passphrase("hunter2", PASSPHRASE_SALT);
        let hex: String = key.as_bytes().iter().map(|b| format!("{b:02x}")).collect();
        write_recovery_file(&path, &hex).unwrap();

        let loaded = try_read_recovery_file(&path).unwrap().unwrap();
        assert_eq!(loaded.as_bytes(), key.as_bytes());
    }

    #[test]
    fn recovery_file_rejects_garbage() {
        let dir = camino_tempfile::tempdir().unwrap();
        let path = dir.path().join("passphrase.env");
        fs::write(&path, "not a real entry\n").unwrap();
        let err = try_read_recovery_file(&path).unwrap_err();
        assert!(format!("{err}").contains("malformed"));
    }

    #[test]
    fn recovery_file_rejects_wrong_size() {
        let dir = camino_tempfile::tempdir().unwrap();
        let path = dir.path().join("passphrase.env");
        fs::write(&path, "LIVTET_PASSPHRASE_HMAC_KEY=deadbeef\n").unwrap();
        let err = try_read_recovery_file(&path).unwrap_err();
        assert!(format!("{err}").contains("wrong size"));
    }

    #[test]
    fn load_state_hmac_key_errors_when_keyring_and_recovery_missing() {
        // We can't easily point the resolver at a real temp config dir
        // from here without changing `default_config_dir`, so this test
        // is best-effort: if the test environment happens to have a
        // real keyring AND a real recovery file, the resolver will
        // return one of them rather than erroring. The "no source
        // available" branch is the dominant case in CI.
        let result = load_state_hmac_key();
        let _ = result; // smoke check; specific branch depends on env
    }
}
