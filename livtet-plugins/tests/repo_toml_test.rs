use livtet_plugins::repository::repo_toml::{
    RepoSection, RepoToml, SigningSection, now_iso, parse_repo_toml, render_repo_toml,
};

fn sample_toml() -> RepoToml {
    RepoToml {
        format_version: 1,
        repo: RepoSection {
            name: "olamaelcu".to_string(),
            url: "https://plugins.livtet.olamaelcu.net".to_string(),
            description: Some("Livtet plugins from Olamaelcu".to_string()),
            maintainer: Some("Olamaelcu <plugins@livtet.olamaelcu.net>".to_string()),
        },
        signing: SigningSection {
            key_label: "olamaelcu".to_string(),
            key_fingerprint: "SHA256:abc123def456".to_string(),
        },
    }
}

#[test]
fn test_repo_toml_round_trip() {
    let original = sample_toml();
    let rendered = render_repo_toml(&original);
    let parsed = parse_repo_toml(&rendered).unwrap();
    assert_eq!(parsed.repo.name, "olamaelcu");
    assert_eq!(parsed.repo.url, "https://plugins.livtet.olamaelcu.net");
    assert_eq!(parsed.signing.key_label, "olamaelcu");
    assert_eq!(parsed.signing.key_fingerprint, "SHA256:abc123def456");
}

#[test]
fn test_repo_toml_rejects_wrong_fingerprint_prefix() {
    let mut toml = sample_toml();
    toml.signing.key_fingerprint = "not-a-fingerprint".to_string();
    let rendered = render_repo_toml(&toml);
    let result = parse_repo_toml(&rendered);
    assert!(result.is_err());
}

#[test]
fn test_repo_toml_rejects_wrong_format_version() {
    let text = r#"
format_version = 99
[repo]
name = "x"
url = "https://example.com"
[signing]
key_label = "x"
key_fingerprint = "SHA256:abc"
"#;
    let result = parse_repo_toml(text);
    assert!(result.is_err());
}

#[test]
fn test_repo_toml_rejects_empty_name() {
    let text = r#"
format_version = 1
[repo]
name = ""
url = "https://example.com"
[signing]
key_label = "x"
key_fingerprint = "SHA256:abc"
"#;
    let result = parse_repo_toml(text);
    assert!(result.is_err());
}

#[test]
fn test_repo_toml_rejects_empty_url() {
    let text = r#"
format_version = 1
[repo]
name = "x"
url = ""
[signing]
key_label = "x"
key_fingerprint = "SHA256:abc"
"#;
    let result = parse_repo_toml(text);
    assert!(result.is_err());
}

#[test]
fn test_now_iso_returns_rfc3339_string() {
    let s = now_iso();
    assert!(s.contains('T'), "expected RFC3339 timestamp with 'T': {s}");
    assert!(s.contains(':'), "expected RFC3339 timestamp with ':': {s}");
}
