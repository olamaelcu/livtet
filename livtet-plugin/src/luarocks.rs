use camino::{Utf8Path, Utf8PathBuf};

use crate::error::PluginError;

const ROCK_TREE_SUBDIR: &str = "lua_modules";
const INSTALLED_MARKER_FILENAME: &str = ".installed";
const LUA_VERSION: &str = "5.4";

/// Composed environment for a spawned Lua child process. Set
/// `LUA_PATH` and `LUA_CPATH` from these strings so the child's
/// `require` resolves modules installed under our rock tree first,
/// then falls back to the Lua stdlib (the trailing `;;`).
#[derive(Debug, Clone)]
pub struct LuarocksEnv {
    pub lua_path: String,
    pub lua_cpath: String,
}

/// Probe the system for `luarocks`. Returns the absolute path if
/// found, `None` if not. Used by the host to decide whether to
/// surface a friendly "luarocks not installed" hint versus a
/// hard install failure.
pub fn detect_luarocks() -> Option<Utf8PathBuf> {
    which("luarocks")
}

/// Run `luarocks install --tree <app_data_dir>/lua_modules <rock>`
/// for each rock in `rocks`. Idempotent: writes a marker file
/// `<app_data_dir>/lua_modules/.installed` with the rock names
/// that have been successfully installed. Skips rocks already in
/// the marker.
///
/// `app_data_dir` is the per-app data dir
/// (e.g. `~/.local/share/net.olamaelcu.livtet/`).
/// `rocks` is the list of luarocks rock names from
/// `PluginMeta.rocks`.
///
/// Returns `Ok(())` if installation succeeded or was a no-op.
/// Returns `Err(PluginError::Luarocks(...))` on failure.
pub async fn ensure_rocks_installed(
    app_data_dir: &Utf8Path,
    rocks: &[String],
) -> Result<(), PluginError> {
    if rocks.is_empty() {
        return Ok(());
    }

    let luarocks_bin = detect_luarocks().ok_or_else(|| {
        PluginError::Luarocks(
            "luarocks was not found on PATH; install it (https://luarocks.org) to use plugins that declare `rocks` in their manifest".to_string(),
        )
    })?;

    let tree = app_data_dir.join(ROCK_TREE_SUBDIR);
    let marker = tree.join(INSTALLED_MARKER_FILENAME);

    fs_err::tokio::create_dir_all(&tree).await.map_err(|e| {
        PluginError::Luarocks(format!("failed to create luarocks tree {}: {e}", tree))
    })?;

    let already_installed = read_marker(&marker).await?;
    let mut newly_installed: Vec<String> = Vec::new();

    for rock in rocks {
        if already_installed.contains(rock) {
            continue;
        }

        install_one_rock(&luarocks_bin, &tree, rock).await?;
        newly_installed.push(rock.clone());
    }

    if !newly_installed.is_empty() {
        append_marker(&marker, &newly_installed).await?;
    }

    Ok(())
}

/// Build the LUA_PATH / LUA_CPATH strings for a given rock tree,
/// suitable for setting on the child process's environment.
/// Includes the standard Lua fallback paths (trailing `;;`) so
/// stdlib (e.g. `require("socket")` if Lua was built with
/// `luasocket`) still works.
pub fn build_env(app_data_dir: &Utf8Path) -> LuarocksEnv {
    let tree = app_data_dir.join(ROCK_TREE_SUBDIR);
    let share_glob = format!("{tree}/share/lua/{LUA_VERSION}/?.lua");
    let share_init_glob = format!("{tree}/share/lua/{LUA_VERSION}/?/init.lua");
    let lua_path = format!("{share_glob};{share_init_glob};;");
    let lib_glob = format!("{tree}/lib/lua/{LUA_VERSION}/?.so");
    let loadall = format!("{tree}/lib/lua/{LUA_VERSION}/loadall.so");
    let lua_cpath = format!("{lib_glob};{loadall};;");
    LuarocksEnv {
        lua_path,
        lua_cpath,
    }
}

async fn install_one_rock(
    luarocks_bin: &Utf8Path,
    tree: &Utf8Path,
    rock: &str,
) -> Result<(), PluginError> {
    let mut cmd = tokio::process::Command::new(luarocks_bin);
    cmd.arg(format!("--lua-version={LUA_VERSION}"))
        .arg(format!("--tree={tree}"))
        .arg("install")
        .arg(rock)
        .stdin(std::process::Stdio::null());

    let output = cmd.output().await.map_err(|e| {
        PluginError::Luarocks(format!(
            "failed to spawn `{}` for rock `{rock}`: {e}",
            luarocks_bin
        ))
    })?;

    if !output.status.success() {
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(PluginError::Luarocks(format!(
            "luarocks install `{rock}` failed (status {}): {}{}{}",
            output.status,
            stderr.trim(),
            if stderr.is_empty() { "" } else { "\n" },
            stdout.trim(),
        )));
    }

    Ok(())
}

async fn read_marker(marker: &Utf8Path) -> Result<std::collections::HashSet<String>, PluginError> {
    if !marker.exists() {
        return Ok(std::collections::HashSet::new());
    }
    let body = fs_err::tokio::read_to_string(marker).await.map_err(|e| {
        PluginError::Luarocks(format!(
            "failed to read luarocks install marker {}: {e}",
            marker
        ))
    })?;
    Ok(body
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_string)
        .collect())
}

async fn append_marker(marker: &Utf8Path, new_rocks: &[String]) -> Result<(), PluginError> {
    use tokio::io::AsyncWriteExt;
    let mut file = fs_err::tokio::OpenOptions::new()
        .create(true)
        .append(true)
        .open(marker)
        .await
        .map_err(|e| {
            PluginError::Luarocks(format!(
                "failed to open luarocks install marker {} for append: {e}",
                marker
            ))
        })?;
    for rock in new_rocks {
        file.write_all(rock.as_bytes()).await.map_err(|e| {
            PluginError::Luarocks(format!(
                "failed to write to luarocks install marker {}: {e}",
                marker
            ))
        })?;
        file.write_all(b"\n").await.map_err(|e| {
            PluginError::Luarocks(format!(
                "failed to write newline to luarocks install marker {}: {e}",
                marker
            ))
        })?;
    }
    Ok(())
}

fn which<S: AsRef<str>>(name: S) -> Option<Utf8PathBuf> {
    which::which(name.as_ref())
        .ok()
        .and_then(|p| Utf8PathBuf::from_path_buf(p).ok())
}
