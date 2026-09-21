use magnus::Error;

pub fn paths() -> Result<magnus::RHash, Error> {
    let ruby = magnus::Ruby::get().unwrap();
    let data = livtet_core::paths::data_dir()
        .map(|p| p.to_string())
        .unwrap_or_default();
    let config = livtet_core::paths::config_dir()
        .map(|p| p.to_string())
        .unwrap_or_default();
    let logs = livtet_core::paths::logs_dir().to_string();
    let hash = ruby.hash_new();
    hash.aset("bundle", livtet_core::paths::BUNDLE_ID)?;
    hash.aset("data", data)?;
    hash.aset("config", config)?;
    hash.aset("logs", logs)?;
    Ok(hash)
}
