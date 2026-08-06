/*!
`[lint]`, and the `selene.toml` a project probably already has.

Same bargain as the formatter's config: a project already linting with selene
should be able to point larvae at it and get its own settings, so `selene.toml`
is read directly and `[lint]` in `larvae.toml` layers over it.
*/

use std::collections::BTreeMap;
use std::path::Path;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

/// How loudly a lint speaks, spelled as selene spells it
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Level {
    /// Off
    Allow,
    /// Reported, exit code unaffected
    #[default]
    Warn,
    /// Reported, and the run fails
    Deny,
}

/// Which set of globals a bare name is checked against
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum StdLib {
    /// Luau's own globals only
    Luau,
    /// Luau plus the Roblox API surface
    #[default]
    Roblox,
    /// Nothing, so every bare name is undefined
    None,
}

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
#[serde(deny_unknown_fields, rename_all = "snake_case")]
pub struct LintConfig {
    /// Per lint level, keyed by the lint's name
    #[serde(default)]
    pub rules: BTreeMap<String, Level>,

    /// Per lint settings, handed to the lint that asked for them
    #[serde(default)]
    pub options: BTreeMap<String, toml::Value>,

    /// Which globals exist before this file runs
    #[serde(default)]
    pub std: StdLib,

    /// Extra globals the project defines, beyond the standard library
    #[serde(default)]
    pub globals: Vec<String>,
}

impl LintConfig {
    /// The level for one lint, its own default unless the project says otherwise
    pub fn level_for(&self, name: &str, default: Level) -> Level {
        self.rules.get(name).copied().unwrap_or(default)
    }

    /// One lint's settings, deserialised into whatever shape it asked for
    pub fn options_for<T: for<'de> Deserialize<'de> + Default>(&self, name: &str) -> T {
        self.options
            .get(name)
            .and_then(|v| v.clone().try_into().ok())
            .unwrap_or_default()
    }

    /*
    Read `selene.toml` if there is one, then lay `[lint]` over it.

    selene puts levels under `[rules]` and per lint settings under `[config]`,
    which is the same information under different names, so the file is
    translated rather than parsed into a second type.
    */
    pub fn discover(root: &Path, larvae: Option<&toml::Value>) -> Result<Self> {
        let mut config = selene_file(root)?.unwrap_or_default();

        if let Some(value) = larvae {
            let over: Self = value.clone().try_into().context("[lint]")?;

            config.rules.extend(over.rules);
            config.options.extend(over.options);
            config.globals.extend(over.globals);

            if value.get("std").is_some() {
                config.std = over.std;
            }
        }

        Ok(config)
    }
}

#[derive(Deserialize)]
struct SeleneFile {
    #[serde(default)]
    rules: BTreeMap<String, Level>,
    #[serde(default)]
    config: BTreeMap<String, toml::Value>,
    #[serde(default)]
    std: Option<String>,
}

fn selene_file(root: &Path) -> Result<Option<LintConfig>> {
    let path = root.join("selene.toml");

    if !path.exists() {
        return Ok(None);
    }

    let text = std::fs::read_to_string(&path)
        .with_context(|| format!("cannot read {}", crate::ui::rel(&path)))?;

    let file: SeleneFile = toml::from_str(&text).context("in selene.toml")?;

    /*
    selene's `std` names a library file and can chain them with `+`, as in
    `roblox+testez`. Only the first name is read, and only the two larvae
    ships are recognised, because a chain naming a file we do not have would
    otherwise silently mean "no globals at all" and bury the file in
    undefined_variable.
    */
    let std = match file.std.as_deref().and_then(|s| s.split('+').next()) {
        Some("luau" | "lua51" | "lua52" | "lua53") => StdLib::Luau,

        Some("roblox") => StdLib::Roblox,

        Some(_) | None => StdLib::default(),
    };

    Ok(Some(LintConfig {
        rules: file.rules,
        options: file.config,
        std,
        globals: Vec::new(),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_lint_uses_its_own_default_until_the_project_says_otherwise() {
        let mut cfg = LintConfig::default();

        assert_eq!(cfg.level_for("unused_variable", Level::Warn), Level::Warn);

        cfg.rules.insert("unused_variable".into(), Level::Allow);

        assert_eq!(cfg.level_for("unused_variable", Level::Warn), Level::Allow);
    }

    #[test]
    fn a_selene_config_is_read_as_written() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("selene.toml"),
            "std = \"roblox\"\n\n[rules]\nunused_variable = \"allow\"\nshadowing = \"deny\"\n",
        )
        .unwrap();

        let cfg = LintConfig::discover(dir.path(), None).unwrap();

        assert_eq!(cfg.std, StdLib::Roblox);
        assert_eq!(cfg.rules["unused_variable"], Level::Allow);
        assert_eq!(cfg.rules["shadowing"], Level::Deny);
    }

    #[test]
    fn selene_per_lint_settings_come_across() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("selene.toml"),
            "[config]\nhigh_cyclomatic_complexity = { maximum_complexity = 40 }\n",
        )
        .unwrap();

        let cfg = LintConfig::discover(dir.path(), None).unwrap();

        assert!(cfg.options.contains_key("high_cyclomatic_complexity"));
    }

    /// A chained std naming a library we do not ship must not mean "no globals"
    #[test]
    fn an_unknown_std_falls_back_rather_than_emptying_the_globals() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("selene.toml"), "std = \"roblox+testez\"\n").unwrap();

        assert_eq!(LintConfig::discover(dir.path(), None).unwrap().std, StdLib::Roblox);

        std::fs::write(dir.path().join("selene.toml"), "std = \"something_else\"\n").unwrap();

        assert_eq!(
            LintConfig::discover(dir.path(), None).unwrap().std,
            StdLib::default()
        );
    }

    #[test]
    fn larvae_config_layers_over_selene() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("selene.toml"),
            "[rules]\nunused_variable = \"allow\"\nshadowing = \"allow\"\n",
        )
        .unwrap();

        let over = toml::from_str::<toml::Value>("[rules]\nshadowing = \"deny\"").unwrap();
        let cfg = LintConfig::discover(dir.path(), Some(&over)).unwrap();

        assert_eq!(cfg.rules["shadowing"], Level::Deny, "larvae should win");
        assert_eq!(
            cfg.rules["unused_variable"],
            Level::Allow,
            "and leave the rest of selene alone"
        );
    }

    #[test]
    fn options_deserialise_into_whatever_a_lint_asked_for() {
        #[derive(Deserialize, Default, PartialEq, Debug)]
        struct Opts {
            maximum: usize,
        }

        let mut cfg = LintConfig::default();
        cfg.options.insert(
            "thing".into(),
            toml::from_str::<toml::Value>("maximum = 7").unwrap(),
        );

        assert_eq!(cfg.options_for::<Opts>("thing"), Opts { maximum: 7 });

        // an absent or malformed table falls back rather than failing the run
        assert_eq!(cfg.options_for::<Opts>("missing"), Opts::default());
    }
}
