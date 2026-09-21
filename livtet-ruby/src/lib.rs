#![allow(unsafe_code)]

mod database;
mod paths;

use magnus::{Error, Ruby, function, method, prelude::*};

#[magnus::init(name = "livtet")]
fn init() -> Result<(), Error> {
    let ruby = Ruby::get().unwrap();
    let livtet = ruby.define_module("Livtet")?;
    livtet.define_module_function("paths", function!(paths::paths, 0))?;
    let std_err: magnus::RClass = ruby.eval("StandardError")?;
    livtet.define_class("Error", std_err)?;
    let db_class = livtet.define_class("Database", ruby.class_object())?;
    db_class.define_singleton_method("native_open", function!(database::db_open, 1))?;
    db_class.define_method("native_path", method!(database::db_path, 0))?;
    db_class.define_method("native_close", method!(database::db_close, 0))?;
    db_class.define_method("native_closed", method!(database::db_closed, 0))?;
    Ok(())
}
