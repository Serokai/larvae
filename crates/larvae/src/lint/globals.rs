/*!
Which names exist before a file runs.

`undefined_variable` is only as good as this list, and a list that is missing
something turns a working file into a wall of false reports, so the bias
throughout is to include a name rather than leave it out. A global that exists
and is not listed costs a wrong warning; one listed that does not exist costs a
warning nobody was going to get anyway.

selene keeps these in TOML files a project can extend or replace. larvae ships
the two that cover almost everyone as static slices instead, because a lookup
that has to parse a file cannot run on every keystroke, and a project with its
own globals lists them under `[lint] globals` rather than authoring a library.
*/

use super::config::StdLib;

/// Luau's own globals, which every Luau host provides
pub const LUAU: &[&str] = &[
    "_G",
    "_VERSION",
    "assert",
    "bit32",
    "buffer",
    "collectgarbage",
    "coroutine",
    "debug",
    "error",
    "getfenv",
    "getmetatable",
    "ipairs",
    "loadstring",
    "math",
    "newproxy",
    "next",
    "os",
    "pairs",
    "pcall",
    "print",
    "rawequal",
    "rawget",
    "rawlen",
    "rawset",
    "require",
    "select",
    "setfenv",
    "setmetatable",
    "string",
    "table",
    "tonumber",
    "tostring",
    "type",
    "typeof",
    "unpack",
    "utf8",
    "vector",
    "warn",
    "xpcall",
];

/// What Roblox adds on top, the data types and the globals its scripts get
pub const ROBLOX: &[&str] = &[
    // the entry points
    "game",
    "plugin",
    "script",
    "shared",
    "workspace",
    // scheduling, including the deprecated forms so they are not also undefined
    "DebuggerManager",
    "PluginManager",
    "UserSettings",
    "delay",
    "elapsedTime",
    "gcinfo",
    "printidentity",
    "settings",
    "spawn",
    "stats",
    "task",
    "tick",
    "time",
    "version",
    "wait",
    // the data types
    "Axes",
    "BrickColor",
    "CFrame",
    "CatalogSearchParams",
    "Color3",
    "ColorSequence",
    "ColorSequenceKeypoint",
    "Content",
    "DateTime",
    "DockWidgetPluginGuiInfo",
    "Enum",
    "Faces",
    "FloatCurveKey",
    "Font",
    "Instance",
    "NumberRange",
    "NumberSequence",
    "NumberSequenceKeypoint",
    "OverlapParams",
    "PathWaypoint",
    "PhysicalProperties",
    "Random",
    "Ray",
    "RaycastParams",
    "Rect",
    "Region3",
    "Region3int16",
    "RotationCurveKey",
    "SecurityCapabilities",
    "SharedTable",
    "TweenInfo",
    "UDim",
    "UDim2",
    "Vector2",
    "Vector2int16",
    "Vector3",
    "Vector3int16",
];

/// Whether this name exists before the file runs
pub fn has(std: StdLib, name: &str) -> bool {
    match std {
        StdLib::None => false,

        StdLib::Luau => LUAU.binary_search(&name).is_ok(),

        StdLib::Roblox => LUAU.binary_search(&name).is_ok() || ROBLOX.contains(&name),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `has` binary searches Luau's list, which only works if it stays sorted
    #[test]
    fn the_luau_list_is_sorted() {
        let mut sorted = LUAU.to_vec();
        sorted.sort_unstable();

        assert_eq!(LUAU, sorted.as_slice());
    }

    #[test]
    fn no_name_is_listed_twice() {
        for list in [LUAU, ROBLOX] {
            let mut seen = list.to_vec();
            seen.sort_unstable();
            let before = seen.len();
            seen.dedup();

            assert_eq!(seen.len(), before);
        }
    }

    /// Roblox adds to Luau rather than replacing it
    #[test]
    fn the_two_lists_do_not_overlap() {
        for name in ROBLOX {
            assert!(!LUAU.contains(name), "{name} is in both lists");
        }
    }

    #[test]
    fn luau_globals_are_found_under_both_libraries() {
        for name in ["print", "pairs", "table", "typeof"] {
            assert!(has(StdLib::Luau, name), "{name}");
            assert!(has(StdLib::Roblox, name), "{name}");
        }
    }

    #[test]
    fn roblox_globals_are_found_only_under_roblox() {
        for name in ["game", "Instance", "task", "Vector3"] {
            assert!(has(StdLib::Roblox, name), "{name}");
            assert!(!has(StdLib::Luau, name), "{name}");
        }
    }

    #[test]
    fn nothing_exists_under_none() {
        assert!(!has(StdLib::None, "print"));
        assert!(!has(StdLib::None, "game"));
    }

    #[test]
    fn a_name_nobody_defines_is_not_found() {
        assert!(!has(StdLib::Roblox, "definitelyNotAGlobal"));
    }

    /*
    The deprecated scheduling globals still exist at runtime, so leaving them
    out would report working code as undefined. Saying they are deprecated is
    a different lint's job.
    */
    #[test]
    fn deprecated_globals_are_still_globals() {
        for name in ["wait", "spawn", "delay", "tick"] {
            assert!(has(StdLib::Roblox, name), "{name}");
        }
    }
}
