fn main() {
    println!("cargo:rerun-if-env-changed=LIVTET_BUNDLED_KEY_PATH");
    let key_text = std::env::var("LIVTET_BUNDLED_KEY_PATH")
        .ok()
        .and_then(|p| std::fs::read_to_string(&p).ok())
        .unwrap_or_default();
    println!("cargo:rustc-env=LIVTET_BUNDLED_SIGNER_PUB_TEXT={}", key_text);
}
