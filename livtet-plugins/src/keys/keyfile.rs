use camino::Utf8Path;
use fs_err as fs;
use minisign::KeyPair;

use crate::{
    archive::error::ArchiveError,
    keys::{fingerprint, passphrase::resolve_passphrase},
    types::KeygenReport,
};

pub fn keygen(
    base_dir: &Utf8Path,
    label: &str,
    no_passphrase: bool,
) -> Result<KeygenReport, ArchiveError> {
    if label.is_empty() {
        return Err(ArchiveError::Key("label cannot be empty".to_string()));
    }
    fs::create_dir_all(base_dir).map_err(ArchiveError::Io)?;

    let key_path = base_dir.join(format!("{label}.key"));
    let pubkey_path = base_dir.join(format!("{label}.pub"));

    if no_passphrase {
        let kp = KeyPair::generate_unencrypted_keypair()
            .map_err(|e| ArchiveError::Key(format!("minisign keygen: {e}")))?;
        let sk_text = kp
            .sk
            .to_box(Some(&format!("livtet signing key {label}")))
            .map_err(|e| ArchiveError::Key(format!("minisign sk to_box: {e}")))?
            .to_string();
        let pk_text = kp
            .pk
            .to_box()
            .map_err(|e| ArchiveError::Key(format!("minisign pk to_box: {e}")))?
            .to_string();
        fs::write(&key_path, sk_text).map_err(ArchiveError::Io)?;
        fs::write(&pubkey_path, pk_text).map_err(ArchiveError::Io)?;
    } else {
        let passphrase = resolve_passphrase(false, Some("LIVTET_KEY_PASSPHRASE"), false)?.0;
        let comment = format!("livtet signing key {label}");
        let mut pk_buf: Vec<u8> = Vec::new();
        let mut sk_buf: Vec<u8> = Vec::new();
        KeyPair::generate_and_write_encrypted_keypair(
            &mut pk_buf,
            &mut sk_buf,
            Some(&comment),
            Some(passphrase),
        )
        .map_err(|e| ArchiveError::Key(format!("minisign encrypted keygen: {e}")))?;
        fs::write(&key_path, &sk_buf).map_err(ArchiveError::Io)?;
        fs::write(&pubkey_path, &pk_buf).map_err(ArchiveError::Io)?;
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Ok(meta) = fs::metadata(&key_path) {
            let mut perms = meta.permissions();
            perms.set_mode(0o600);
            let _ = fs::set_permissions(&key_path, perms);
        }
    }

    let pubkey_text = fs::read_to_string(&pubkey_path).map_err(ArchiveError::Io)?;
    let verifying_key = crate::keys::signing::parse_pubkey_text(&pubkey_text)?;

    Ok(KeygenReport {
        label: label.to_string(),
        key_path,
        pubkey_path,
        fingerprint: fingerprint(&verifying_key),
        encrypted: !no_passphrase,
    })
}
