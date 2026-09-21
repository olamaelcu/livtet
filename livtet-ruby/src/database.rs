use std::sync::{Mutex, OnceLock};

use livtet_core::SharedState;
use livtet_data::migrator::Kind;
use magnus::{Error, Ruby, Value, prelude::*};

static RUNTIME: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
static DB_HANDLE: Mutex<Option<(String, SharedState)>> = Mutex::new(None);

fn runtime() -> &'static tokio::runtime::Runtime {
    RUNTIME
        .get_or_init(|| tokio::runtime::Runtime::new().expect("livtet: failed to create tokio runtime"))
}

fn ruby_error(msg: String) -> Error {
    let ruby = Ruby::get().unwrap();
    let cls = ruby
        .eval::<magnus::RClass>("Livtet::Error")
        .ok()
        .and_then(|c| magnus::ExceptionClass::from_value(c.as_value()))
        .unwrap_or_else(|| ruby.exception_standard_error());
    Error::new(cls, msg)
}

fn default_db_path() -> Result<String, Error> {
    livtet_core::paths::data_dir()
        .map(|p| format!("{p}/livtet.db"))
        .ok_or_else(|| ruby_error("livtet: cannot determine default data directory".to_string()))
}

pub fn db_open(path: Value) -> Result<bool, Error> {
    if DB_HANDLE.lock().unwrap().is_some() {
        return Err(ruby_error("livtet: database already open".to_string()));
    }
    let db_path: String = if path.is_nil() {
        default_db_path()?
    } else {
        let s: String = String::try_convert(path)
            .map_err(|_| ruby_error("livtet: path must be a String".to_string()))?;
        if s.is_empty() {
            return Err(ruby_error("livtet: path must not be empty".to_string()));
        }
        s
    };
    if let Some(parent) = std::path::Path::new(&db_path).parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)
                .map_err(|e| ruby_error(format!("livtet: cannot create database directory: {e}")))?;
        }
    }
    let state = runtime()
        .block_on(SharedState::connect(&db_path, &[Kind::Business]))
        .map_err(|e| ruby_error(format!("livtet: failed to open database: {e}")))?;
    *DB_HANDLE.lock().unwrap() = Some((db_path, state));
    Ok(true)
}

pub fn db_path(_self: Value) -> Result<String, Error> {
    DB_HANDLE
        .lock()
        .unwrap()
        .as_ref()
        .map(|(p, _)| p.clone())
        .ok_or_else(|| ruby_error("livtet: database is not open".to_string()))
}

pub fn db_close(_self: Value) -> Result<bool, Error> {
    let taken = DB_HANDLE.lock().unwrap().take();
    match taken {
        Some((_, state)) => {
            runtime()
                .block_on(state.optimize_and_close())
                .map_err(|e| ruby_error(format!("livtet: failed to close database: {e}")))?;
            Ok(true)
        }
        None => Err(ruby_error("livtet: database is not open".to_string())),
    }
}

pub fn db_closed(_self: Value) -> Result<bool, Error> {
    Ok(DB_HANDLE.lock().unwrap().is_none())
}
