use std::assert_matches;

use camino::Utf8PathBuf;
use livtet_plugin::{
    archive::error::ArchiveError,
    types::{InstallReport, RepositoryAddResult, RepositoryUpdateResult, TrustedKeySource},
};

#[test]
fn test_install_report_serde_roundtrip() {
    let r = InstallReport {
        id: "openlibrary".to_string(),
        version: "1.0.0".to_string(),
        signer_label: "olamaelcu".to_string(),
        signer_fingerprint: "SHA256:abc".to_string(),
        trusted: true,
        replaced_versions: vec!["0.9.0".to_string()],
        warnings: vec![],
        install_path: Utf8PathBuf::from(
            "/home/user/.local/share/net.olamaelcu.livtet/providers/openlibrary/1.0.0",
        ),
    };
    let json = serde_json::to_string(&r).unwrap();
    let back: InstallReport = serde_json::from_str(&json).unwrap();
    assert_eq!(r.id, back.id);
    assert_eq!(r.replaced_versions, back.replaced_versions);
}

#[test]
fn test_repository_add_result_tagu() {
    let needs_tofu = RepositoryAddResult::NeedsTofuConfirmation {
        name: "olamaelcu".to_string(),
        url: "https://plugins.livtet.olamaelcu.net".to_string(),
        fingerprint: "SHA256:abc".to_string(),
    };
    let json = serde_json::to_string(&needs_tofu).unwrap();
    assert!(json.contains("\"kind\":\"needs_tofu_confirmation\""));
    let back: RepositoryAddResult = serde_json::from_str(&json).unwrap();
    match back {
        RepositoryAddResult::NeedsTofuConfirmation {
            name,
            url,
            fingerprint,
        } => {
            assert_eq!(name, "olamaelcu");
            assert_eq!(url, "https://plugins.livtet.olamaelcu.net");
            assert_eq!(fingerprint, "SHA256:abc");
        }
        _ => panic!("wrong variant"),
    }
}

#[test]
fn test_repository_update_result_key_changed() {
    let result = RepositoryUpdateResult::KeyChanged {
        name: "olamaelcu".to_string(),
        old_fingerprint: "SHA256:old".to_string(),
        new_fingerprint: "SHA256:new".to_string(),
    };
    let json = serde_json::to_string(&result).unwrap();
    assert!(json.contains("\"kind\":\"key_changed\""));
    assert!(json.contains("olamaelcu"));
}

#[test]
fn test_trusted_key_source_serde() {
    let s = TrustedKeySource::Builtin;
    let json = serde_json::to_string(&s).unwrap();
    assert_eq!(json, "\"builtin\"");
    let back: TrustedKeySource = serde_json::from_str(&json).unwrap();
    assert_matches!(back, TrustedKeySource::Builtin);
}

#[test]
fn test_archive_error_display() {
    let e = ArchiveError::UntrustedKey {
        fingerprint: "SHA256:abc".to_string(),
    };
    assert!(e.to_string().contains("SHA256:abc"));

    let e = ArchiveError::PassphraseRequired {
        key_path: Utf8PathBuf::from("/tmp/test.key"),
    };
    assert!(e.to_string().contains("/tmp/test.key"));
}
