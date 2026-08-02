//! Lua standard library for the livtet plugin system.
//!
//! Provides built-in Lua rocks (libraries) that plugins can load
//! via `host.require(name)`. Each rock is embedded at compile time
//! as a static string, so there is no runtime file I/O.

/// Look up the Lua source for a named rock.
///
/// Returns `None` if the rock is not bundled in this build.
/// The returned string is `'static` because rocks are compiled
/// into the binary via `include_str!()`.
pub fn get_rock_source(name: &str) -> Option<&'static str> {
    ROCK_REGISTRY
        .iter()
        .find_map(|(n, src)| if *n == name { Some(*src) } else { None })
}

/// List every rock name bundled in this build.
pub fn list_rocks() -> Vec<&'static str> {
    ROCK_REGISTRY.iter().map(|(name, _)| *name).collect()
}

// --- Compile-time registry --------------------------------------------------

/// Macro that builds a lookup table of rock names to their embedded source.
/// Uses a simple static slice of pairs and linear search, since the set is
/// small (single-digit rocks) and we want zero extra dependencies.
macro_rules! rock_registry {
    ($($name:literal => $path:literal),* $(,)?) => {
        static ROCK_REGISTRY: &[(&str, &str)] = &[
            $(( $name, include_str!($path) )),*
        ];
    };
}

rock_registry! {
    "dkjson" => "rocks/dkjson.lua",
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn list_rocks_includes_dkjson() {
        let rocks = list_rocks();
        assert!(
            rocks.contains(&"dkjson"),
            "dkjson should be in the registry, got: {rocks:?}"
        );
    }

    #[test]
    fn get_dkjson_returns_source() {
        let source = get_rock_source("dkjson");
        assert!(source.is_some(), "dkjson source should exist");
        let src = source.unwrap();
        assert!(src.contains("dkjson.encode"), "dkjson encode should be in source");
        assert!(src.contains("dkjson.decode"), "dkjson decode should be in source");
    }

    #[test]
    fn get_nonexistent_rock_returns_none() {
        assert_eq!(get_rock_source("no_such_rock"), None);
    }

    #[test]
    fn dkjson_loads_in_mlua() {
        let lua = mlua::Lua::new();
        let source = get_rock_source("dkjson").expect("dkjson should exist");
        let result: mlua::Result<mlua::Value> = lua.load(source).eval();
        assert!(
            result.is_ok(),
            "dkjson should parse as valid Lua: {:?}",
            result.err()
        );
        let val = result.unwrap();
        assert!(val.is_table(), "dkjson should return a table");
        let table = val.as_table().unwrap();
        assert!(
            table.contains_key("encode").unwrap_or(false),
            "dkjson table should have encode"
        );
        assert!(
            table.contains_key("decode").unwrap_or(false),
            "dkjson table should have decode"
        );
    }

    #[test]
    fn dkjson_round_trips() {
        let lua = mlua::Lua::new();
        let source = get_rock_source("dkjson").expect("dkjson should exist");
        let dkjson: mlua::Table = lua.load(source).eval().unwrap();
        let encode: mlua::Function = dkjson.get("encode").unwrap();
        let decode: mlua::Function = dkjson.get("decode").unwrap();

        let original = lua.create_table().unwrap();
        original.set("hello", "world").unwrap();
        original.set("num", 42).unwrap();
        let json: String = encode.call(original.clone()).unwrap();
        let decoded: mlua::Value = decode.call(json).unwrap();
        let decoded_table = decoded.as_table().unwrap();
        assert_eq!(decoded_table.get::<String>("hello").unwrap(), "world");
        assert_eq!(decoded_table.get::<i32>("num").unwrap(), 42);
    }
}
