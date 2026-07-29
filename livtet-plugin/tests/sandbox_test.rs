use std::assert_matches;

use livtet_plugin::mlua::{self, LuaOptions, StdLib, Value};

fn build_sandboxed_lua() -> mlua::Lua {
    let libs = StdLib::COROUTINE | StdLib::TABLE | StdLib::STRING | StdLib::UTF8 | StdLib::MATH;
    let lua = mlua::Lua::new_with(libs, LuaOptions::default()).unwrap();
    let globals = lua.globals();
    let _ = globals.set("os", Value::Nil);
    let _ = globals.set("io", Value::Nil);
    let _ = globals.set("debug", Value::Nil);
    let _ = globals.set("require", Value::Nil);
    lua
}

#[test]
fn test_sandbox_dangerous_globals_are_nil() {
    let lua = build_sandboxed_lua();
    let globals = lua.globals();

    let os: Value = globals.get("os").unwrap_or(Value::Nil);
    assert_matches!(os, Value::Nil);

    let io: Value = globals.get("io").unwrap_or(Value::Nil);
    assert_matches!(io, Value::Nil);

    let debug: Value = globals.get("debug").unwrap_or(Value::Nil);
    assert_matches!(debug, Value::Nil);
}

#[test]
fn test_sandbox_os_execute_raises_error() {
    let lua = build_sandboxed_lua();
    let result: mlua::Result<String> = lua.load("return os.execute('echo pwned')").eval();
    assert!(result.is_err(), "os.execute must not be callable");
}

#[test]
fn test_sandbox_io_open_raises_error() {
    let lua = build_sandboxed_lua();
    let result: mlua::Result<String> = lua.load("return io.open('/etc/passwd')").eval();
    assert!(result.is_err(), "io.open must not be callable");
}

#[test]
fn test_sandbox_memory_limit_enforced() {
    let libs = StdLib::COROUTINE | StdLib::TABLE | StdLib::STRING | StdLib::UTF8 | StdLib::MATH;
    let lua = mlua::Lua::new_with(libs, LuaOptions::default()).unwrap();
    lua.set_memory_limit(1024).unwrap();

    let result: mlua::Result<String> = lua
        .load("local t = {}; for i = 1, 100000 do t[i] = string.rep('x', 1024) end; return 'ok'")
        .eval();
    assert!(result.is_err(), "memory limit should be enforced");
}
