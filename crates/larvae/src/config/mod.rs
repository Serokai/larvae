//! The module loads and validates larvae.toml. An unknown key is a hard error.
//! A planned key gives an error that names its milestone.

use std::collections::HashMap;
use std::path::Path;

use anyhow::{Context, Result, bail};
use serde::Deserialize;

pub mod bundle;
pub mod check;
mod excludes;
pub mod minify;
mod overrides;
mod process;
mod profile;
mod requires;
mod rules;
pub mod worms;

pub use excludes::Excludes;
pub use overrides::{Override, lookup as override_for, parse as parse_overrides};
pub use process::{Input, ProcessConfig, QuoteStyle};
pub use requires::{IndexingStyle, RequiresConfig, RojoConfig, Target};
pub use rules::{
    AppendTextComment, PreserveSideEffects, RemoveAttribute, RemoveCalls, RemoveComments,
    RemoveInterpolatedString, RuleStatus, RulesConfig, rule_status,
};

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    /// Globs that every command skips, relative to the project root
    #[serde(default)]
    pub exclude: Vec<String>,

    /// Globs that cancel the root `exclude`, and only that. An area include
    /// is stronger: it also cancels the exclude of its own area.
    #[serde(default)]
    pub include: Vec<String>,

    /*
    The three keys that almost every project sets, at the root so the first
    line of a config needs no table header. Each one is a short form:
    `input` and `output` for the same keys of `[process]`, and `target` for
    `[requires] target`. `load_profile` folds a set value into its long home
    and refuses a config that writes both spellings of one key. After the
    fold the fields here are None, so the rest of larvae reads one place.
    */
    #[serde(default)]
    input: Option<Input>,

    #[serde(default)]
    output: Option<std::path::PathBuf>,

    #[serde(default)]
    target: Option<Target>,

    #[serde(default)]
    pub aliases: HashMap<String, String>,

    #[serde(default)]
    pub process: ProcessConfig,

    #[serde(default)]
    pub requires: RequiresConfig,

    #[serde(default)]
    pub rojo: RojoConfig,

    #[serde(default)]
    pub rules: RulesConfig,

    /// Compile time constants; larvae replaces the names with literals
    #[serde(default)]
    pub defines: Option<toml::Value>,

    /// Formatter settings, layered over a stylua.toml if the project has one
    #[serde(default)]
    pub fmt: Option<toml::Value>,

    /// Linter settings, layered over a selene.toml if the project has one
    #[serde(default)]
    pub lint: Option<toml::Value>,

    /// The extensions that the project requests, and their settings, see [`worms`]
    #[serde(default)]
    pub worms: Option<toml::Value>,

    /// The `larvae check` gate, see [`check`]
    #[serde(default)]
    pub check: check::CheckConfig,

    /// The `larvae bundle` output, see [`bundle`]
    #[serde(default)]
    pub bundle: bundle::BundleConfig,

    /// Tuning for `generator = "dense"`, see [`minify`]
    #[serde(default)]
    pub minify: minify::MinifyConfig,

    // Parsed so the error can name the new location and not say "unknown key".
    #[serde(default)]
    config: Option<toml::Value>,
}

impl Config {
    pub fn load(path: &Path) -> Result<Self> {
        Self::load_profile(path, None)
    }

    /// Load a config, and merge `[profile.<name>]` over it when the caller requests one
    pub fn load_profile(path: &Path, profile: Option<&str>) -> Result<Self> {
        let text = std::fs::read_to_string(path)
            .with_context(|| format!("failed to read {}", crate::ui::rel(path)))?;

        let mut raw: toml::Value = toml::from_str(&text)
            .with_context(|| format!("invalid config in {}", crate::ui::rel(path)))?;

        if let Some(name) = profile {
            profile::apply(&mut raw, name)
                .with_context(|| format!("--profile {name} in {}", crate::ui::rel(path)))?;
        } else if let Some(table) = raw.as_table_mut() {
            // The caller requested no profile, so the block is inert config.
            table.remove("profile");
        }

        /*
        The short form and the long form of one key must not both appear.
        Serde fills a missing key with its default, so after the parse larvae
        cannot tell a written key from a filled one. The raw table still can.
        */
        for (short, table) in [
            ("input", "process"),
            ("output", "process"),
            ("target", "requires"),
        ] {
            if raw.get(short).is_some() && raw.get(table).and_then(|t| t.get(short)).is_some() {
                bail!(
                    "`{short}` is set twice, at the root and in [{table}]; keep one, they mean the same thing"
                );
            }
        }

        let mut config: Config = raw
            .try_into()
            .with_context(|| format!("invalid config in {}", crate::ui::rel(path)))?;

        if let Some(input) = config.input.take() {
            config.process.input = input;
        }

        if let Some(output) = config.output.take() {
            config.process.output = output;
        }

        if let Some(target) = config.target.take() {
            config.requires.target = target;
        }

        config.validate()?;

        Ok(config)
    }

    /// Load `larvae.toml` from `dir` if the file exists; use the zero config default otherwise
    pub fn load_or_default(dir: &Path) -> Result<Self> {
        Self::load_or_default_profile(dir, None)
    }

    pub fn load_or_default_profile(dir: &Path, profile: Option<&str>) -> Result<Self> {
        let path = dir.join("larvae.toml");

        if path.exists() {
            Self::load_profile(&path, profile)
        } else if let Some(name) = profile {
            bail!("--profile {name} needs a larvae.toml, there is none here")
        } else {
            Ok(Self::default())
        }
    }

    fn validate(&self) -> Result<()> {
        /*
        The settings of a worm sat in a top level [config.<name>] before, one
        table away from the [worms.<name>] entry for the same worm. Now all
        data for one worm lives under one key. So this error names the new
        location and does not report an unknown key.
        */
        if let Some(table) = &self.config {
            let name = table
                .as_table()
                .and_then(|t| t.keys().next().cloned())
                .unwrap_or_else(|| "<name>".to_string());

            bail!(
                "[config.{name}] moved, a worm's settings live with the worm now: write [worms.{name}.config]"
            );
        }

        /*
        [rules] belongs only to larvae. The rules of a worm sit under
        [worms.<name>] rules. So a worm rule cannot shadow a builtin, and this
        check needs no data from worms.
        */
        for name in self.rules.rest.keys() {
            match rule_status(name) {
                Some(RuleStatus::Planned(m)) => bail!(
                    "rule \"{name}\" is not implemented yet, it lands in {m}, remove it for now"
                ),

                Some(RuleStatus::Elsewhere(where_)) => {
                    bail!("\"{name}\" is not a larvae rule, {where_}")
                }

                Some(RuleStatus::Done) => {}

                None => bail!("unknown rule \"{name}\""),
            }
        }

        if let Some(a) = &self.rules.append_text_comment {
            if a.text.is_some() == a.file.is_some() {
                bail!("append_text_comment needs exactly one of `text` or `file`");
            }

            if a.location != "start" && a.location != "end" {
                bail!(
                    "append_text_comment location must be \"start\" or \"end\", got \"{}\"",
                    a.location
                );
            }
        }

        if let Some(r) = &self.rules.remove_interpolated_string {
            let s = r.strategy();

            if s != "string" && s != "tostring" {
                bail!(
                    "remove_interpolated_string strategy must be \"string\" or \"tostring\", got \"{s}\""
                );
            }
        }

        if let Some(a) = &self.rules.remove_attribute {
            for p in a.patterns() {
                if let Err(e) = regex::Regex::new(p) {
                    bail!("invalid remove_attribute match pattern \"{p}\": {e}");
                }
            }
        }

        if let Some(name) = &self.rules.inject_module_path
            && !crate::rules::native::is_ident(name)
        {
            bail!("inject_module_path must be a Luau identifier, got \"{name}\"");
        }

        if let Some(calls) = &self.rules.remove_calls {
            for name in calls.functions() {
                if !name.split('.').all(crate::rules::native::is_ident) {
                    bail!(
                        "remove_calls entry \"{name}\" must be a name or a dotted path of names, ex: \"debug.profilebegin\""
                    );
                }
            }
        }

        if !matches!(
            self.process.generator.as_str(),
            "retain-lines" | "dense" | "readable"
        ) {
            bail!(
                "generator = \"{}\" is not one larvae has; write \"retain-lines\", \"dense\", or \"readable\"",
                self.process.generator
            );
        }

        if self.requires.indexing_style.is_some() && self.requires.target != Target::RobloxInstance
        {
            bail!(
                "requires.indexing_style only applies when requires.target = \"roblox-instance\""
            );
        }

        if let Some(table) = &self.requires.overrides {
            overrides::parse(table)?;
        }

        if let Some(table) = &self.defines
            && let Err(msg) = crate::rules::defines::parse(table)
        {
            bail!("{msg}");
        }

        for (name, value) in &self.aliases {
            validate_alias_name(name)?;

            if value.is_empty() {
                bail!("alias \"{name}\" has an empty value");
            }
        }

        Ok(())
    }

    /// The alias map with RFC case insensitive names (lowercase keys)
    pub fn alias_map(&self) -> HashMap<String, String> {
        self.aliases
            .iter()
            .map(|(k, v)| (k.to_lowercase(), v.clone()))
            .collect()
    }
}

pub fn validate_alias_name(name: &str) -> Result<()> {
    if name.is_empty() {
        bail!("empty alias name");
    }

    let lower = name.to_lowercase();

    if lower == "self" || lower == "game" {
        bail!("alias \"@{name}\" is reserved by Roblox/Luau and cannot be redefined");
    }

    if let Some(bad) = name
        .chars()
        .find(|c| !(c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-')))
    {
        bail!("alias \"@{name}\" contains invalid character {bad:?} (allowed: A-Z a-z 0-9 . _ -)");
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn zero_config_defaults() {
        let c: Config = toml::from_str("").unwrap();
        c.validate().unwrap();

        assert_eq!(c.process.inputs(), vec![PathBuf::from("src")]);
        assert_eq!(c.requires.target, Target::RobloxString);
    }

    #[test]
    fn larvae_rules_load_in_both_forms() {
        let c: Config = toml::from_str(concat!(
            "[rules]\n",
            "remove_calls = { functions = [\"print\", \"debug.profilebegin\"], preserve_arguments_side_effects = false }\n",
            "use_get_service = true\n",
            "dedupe_requires = true\n",
            "inject_module_path = \"MODULE_PATH\"\n",
            "freeze_module = true\n",
        ))
        .unwrap();
        c.validate().unwrap();
        let calls = c.rules.remove_calls.as_ref().unwrap();
        assert_eq!(calls.functions().len(), 2);
        assert!(!calls.preserve_arguments_side_effects());
        // The list form keeps the safe default.
        let c: Config = toml::from_str("[rules]\nremove_calls = [\"print\"]").unwrap();
        c.validate().unwrap();
        assert!(
            c.rules
                .remove_calls
                .as_ref()
                .unwrap()
                .preserve_arguments_side_effects()
        );
    }

    #[test]
    fn generated_names_must_be_identifiers() {
        let c: Config = toml::from_str("[rules]\ninject_module_path = \"not a name\"").unwrap();
        assert!(c.validate().is_err());
        let c: Config = toml::from_str("[rules]\nremove_calls = [\"obj:method\"]").unwrap();
        assert!(c.validate().is_err());
    }

    #[test]
    fn unknown_key_is_hard_error() {
        assert!(toml::from_str::<Config>("[procces]\ninput = \"x\"").is_err());
        assert!(toml::from_str::<Config>("[process]\ninpt = \"x\"").is_err());
    }

    /// The old spelling names the new location; an unknown key error would not
    #[test]
    fn a_top_level_config_table_points_at_the_worm() {
        let c: Config = toml::from_str("[config.xml]\npretty = true").unwrap();
        let err = c.validate().unwrap_err().to_string();

        assert!(err.contains("[worms.xml.config]"), "{err}");
    }

    /// [minify] landed, so a [minify] table is configuration and not an error
    #[test]
    fn a_minify_table_is_read_rather_than_refused() {
        let c: Config = toml::from_str("[minify]\ncolumn_span = 100").unwrap();

        c.validate().unwrap();
        assert_eq!(c.minify.column_span, 100);
        assert!(!c.minify.rename_variables);
    }

    #[test]
    fn every_generator_is_accepted_and_a_typo_names_the_real_ones() {
        for name in ["retain-lines", "dense", "readable"] {
            let c: Config = toml::from_str(&format!("[process]\ngenerator = \"{name}\"")).unwrap();

            c.validate().unwrap();
        }

        let c: Config = toml::from_str("[process]\ngenerator = \"minified\"").unwrap();
        let err = c.validate().unwrap_err().to_string();

        assert!(err.contains("dense") && err.contains("readable"), "{err}");
    }

    /// [check] landed, so a [check] table is configuration and not an error
    #[test]
    fn a_check_table_is_read_rather_than_refused() {
        let c: Config = toml::from_str("[check]\ncycles = \"deny\"").unwrap();

        c.validate().unwrap();
        assert_eq!(c.check.cycles, crate::lint::Level::Deny);
    }

    /// [bundle] landed, so a [bundle] table is configuration and not an error
    #[test]
    fn a_bundle_table_is_read_rather_than_refused() {
        let c: Config = toml::from_str("[bundle]\nentry = \"src/init.luau\"").unwrap();

        c.validate().unwrap();
        assert_eq!(
            c.bundle.entry,
            Some(std::path::PathBuf::from("src/init.luau"))
        );
        assert!(c.bundle.tree_shake);
    }

    #[test]
    fn reserved_alias_rejected() {
        let c: Config = toml::from_str("[aliases]\nself = \"./x\"").unwrap();
        assert!(c.validate().is_err());

        let c: Config = toml::from_str("[aliases]\nGame = \"./x\"").unwrap();
        assert!(c.validate().is_err());
    }

    #[test]
    fn alias_map_is_case_insensitive() {
        let c: Config =
            toml::from_str("[aliases]\nPkg = \"@game/ReplicatedStorage/packages\"").unwrap();
        assert!(c.alias_map().contains_key("pkg"));
    }

    /// The fold runs in `load_profile`, so these tests load from a file.
    fn load_text(text: &str, profile: Option<&str>) -> Result<Config> {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("larvae.toml");

        std::fs::write(&path, text).unwrap();

        Config::load_profile(&path, profile)
    }

    #[test]
    fn the_root_short_forms_fold_into_their_long_homes() {
        let c = load_text(
            "input = \"game\"\noutput = \"out\"\ntarget = \"path\"\n",
            None,
        )
        .unwrap();

        assert_eq!(c.process.inputs(), vec![PathBuf::from("game")]);
        assert_eq!(c.process.output, PathBuf::from("out"));
        assert_eq!(c.requires.target, Target::Path);
    }

    #[test]
    fn both_spellings_of_one_key_are_refused() {
        for text in [
            "input = \"src\"\n\n[process]\ninput = \"src\"\n",
            "output = \"dist\"\n\n[process]\noutput = \"dist\"\n",
            "target = \"path\"\n\n[requires]\ntarget = \"path\"\n",
        ] {
            let err = load_text(text, None).unwrap_err().to_string();

            assert!(err.contains("set twice"), "{err}");
        }
    }

    /// A profile merges before the fold, so a profile can set a short form.
    #[test]
    fn a_profile_can_set_a_short_form() {
        let c = load_text(
            "input = \"src\"\n\n[profile.ship]\ninput = \"game\"\n",
            Some("ship"),
        )
        .unwrap();

        assert_eq!(c.process.inputs(), vec![PathBuf::from("game")]);
    }
}
