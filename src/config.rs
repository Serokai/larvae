//! coldluau.toml loading and validation, unknown keys hard error, planned keys error with their milestone

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde::Deserialize;

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
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

    // parsed but unimplemented, so the error can name the milestone
    #[serde(default)]
    defines: Option<toml::Value>,

    #[serde(default)]
    extensions: Option<toml::Value>,

    #[serde(default)]
    bundle: Option<toml::Value>,

    #[serde(default)]
    minify: Option<toml::Value>,

    #[serde(default)]
    check: Option<toml::Value>,

    #[serde(default)]
    profile: Option<toml::Value>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProcessConfig {
    #[serde(default = "default_input")]
    pub input: PathBuf,

    #[serde(default = "default_output")]
    pub output: PathBuf,

    #[serde(default = "default_include")]
    pub include: Vec<String>,

    #[serde(default)]
    pub exclude: Vec<String>,

    #[serde(default = "default_generator")]
    pub generator: String,

    #[serde(default)]
    pub quotes: QuoteStyle,

    #[serde(default = "default_true")]
    pub cache: bool,

    #[serde(default = "default_cache_dir")]
    pub cache_dir: PathBuf,
}

/// Quote character for require strings in the output
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum QuoteStyle {
    /// Keep whatever the source used
    #[default]
    Preserve,
    Double,
    Single,
}

impl QuoteStyle {
    /// The quote char to emit, preserve defaults to double for generated text
    pub fn char(self) -> char {
        match self {
            QuoteStyle::Single => '\'',
            QuoteStyle::Preserve | QuoteStyle::Double => '"',
        }
    }
}

/*
Builtin transforms, all off by default, every darklua rule name is accepted
so configs port over 1:1, names that are not implemented yet error with the
milestone they land in instead of being silently ignored
*/
#[derive(Debug, Default, Deserialize)]
pub struct RulesConfig {
    #[serde(default)]
    pub const_requires: bool,

    #[serde(default)]
    pub remove_comments: Option<RemoveComments>,

    #[serde(default)]
    pub append_text_comment: Option<AppendTextComment>,

    /// coldluau only, ex: "strict" writes `--!strict` at the top
    #[serde(default)]
    pub add_luau_directive: Option<String>,

    // --- darklua parity, tier one, plain tree edits -----------------------
    #[serde(default)]
    pub remove_method_definition: bool,

    #[serde(default)]
    pub remove_compound_assignment: bool,

    #[serde(default)]
    pub remove_floor_division: bool,

    #[serde(default)]
    pub remove_if_expression: bool,

    #[serde(default)]
    pub remove_method_call: bool,

    #[serde(default)]
    pub convert_index_to_field: bool,

    #[serde(default)]
    pub convert_function_to_assignment: bool,

    #[serde(default)]
    pub convert_luau_number: bool,

    #[serde(default)]
    pub make_assignment_local: bool,

    #[serde(default)]
    pub remove_types: bool,

    #[serde(default)]
    pub remove_attribute: Option<RemoveAttribute>,

    #[serde(default)]
    pub remove_function_call_parens: bool,

    #[serde(default)]
    pub filter_after_early_return: bool,

    #[serde(default)]
    pub remove_interpolated_string: Option<RemoveInterpolatedString>,

    #[serde(default)]
    pub remove_continue: bool,

    #[serde(default)]
    pub remove_empty_do: bool,

    // --- darklua parity, tier two, needs the evaluator --------------------
    #[serde(default)]
    pub remove_assertions: Option<PreserveSideEffects>,

    #[serde(default)]
    pub remove_debug_profiling: Option<PreserveSideEffects>,

    #[serde(default)]
    pub compute_expression: bool,

    #[serde(default)]
    pub remove_unused_if_branch: bool,

    #[serde(default)]
    pub remove_unused_while: bool,

    #[serde(default)]
    pub remove_nil_declaration: bool,

    #[serde(default)]
    pub group_local_assignment: bool,

    #[serde(default)]
    pub convert_local_function_to_assign: bool,

    #[serde(default)]
    pub convert_square_root_call: bool,

    // --- coldluau only, no darklua rule does any of these ---
    /// Statement position calls to drop, ex: ["print", "debug.profilebegin"]
    #[serde(default)]
    pub remove_calls: Option<RemoveCalls>,

    /// `game.Players` becomes `game:GetService("Players")`
    #[serde(default)]
    pub use_get_service: bool,

    /// Top level requires of the same module collapse onto the first one
    #[serde(default)]
    pub dedupe_requires: bool,

    /// Name of a constant holding this file's datamodel path
    #[serde(default)]
    pub inject_module_path: Option<String>,

    /// Wrap a provably safe module return in `table.freeze`
    #[serde(default)]
    pub freeze_module: bool,

    #[serde(flatten)]
    rest: HashMap<String, toml::Value>,
}

/*
`remove_assertions = true` or a table carrying darklua's
`preserve_arguments_side_effects`, shared by remove_debug_profiling because
darklua spells the option the same way in both rules
*/
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum PreserveSideEffects {
    Enabled(bool),
    Options {
        /// Keep calls whose arguments might do something, on by default
        #[serde(default = "default_true")]
        preserve_arguments_side_effects: bool,
    },
}

impl PreserveSideEffects {
    pub fn enabled(&self) -> bool {
        !matches!(self, PreserveSideEffects::Enabled(false))
    }

    pub fn preserve(&self) -> bool {
        match self {
            PreserveSideEffects::Enabled(_) => true,
            PreserveSideEffects::Options {
                preserve_arguments_side_effects,
            } => *preserve_arguments_side_effects,
        }
    }
}

/// `remove_attribute = true` or a table with `match` patterns
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum RemoveAttribute {
    Enabled(bool),
    Options {
        /// Regexes tested against the attribute name, empty means every one
        #[serde(default, rename = "match")]
        patterns: Vec<String>,
    },
}

impl RemoveAttribute {
    pub fn enabled(&self) -> bool {
        !matches!(self, RemoveAttribute::Enabled(false))
    }

    pub fn patterns(&self) -> &[String] {
        match self {
            RemoveAttribute::Enabled(_) => &[],
            RemoveAttribute::Options { patterns } => patterns,
        }
    }
}

/// `remove_interpolated_string = true` or a table with `strategy`
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum RemoveInterpolatedString {
    Enabled(bool),
    Options {
        /// "string" wraps each value in tostring, "tostring" uses `%*`
        #[serde(default = "default_interp_strategy")]
        strategy: String,
    },
}

fn default_interp_strategy() -> String {
    "string".to_string()
}

impl RemoveInterpolatedString {
    pub fn enabled(&self) -> bool {
        !matches!(self, RemoveInterpolatedString::Enabled(false))
    }

    pub fn strategy(&self) -> &str {
        match self {
            RemoveInterpolatedString::Enabled(_) => "string",
            RemoveInterpolatedString::Options { strategy } => strategy,
        }
    }
}

/// `remove_comments = true` or a table with `except` patterns
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum RemoveComments {
    Enabled(bool),
    Options {
        /// Regexes for comments to keep, defaults to Luau directives
        #[serde(default = "default_comment_except")]
        except: Vec<String>,
    },
}

fn default_comment_except() -> Vec<String> {
    // keep --!strict and friends, dropping them changes how Luau runs
    vec!["^--!".to_string()]
}

impl RemoveComments {
    pub fn enabled(&self) -> bool {
        !matches!(self, RemoveComments::Enabled(false))
    }

    pub fn except(&self) -> Vec<String> {
        match self {
            RemoveComments::Enabled(_) => default_comment_except(),
            RemoveComments::Options { except } => except.clone(),
        }
    }
}

/// `remove_calls = ["print"]` or a table that also sets the side effect switch
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum RemoveCalls {
    Functions(Vec<String>),

    Options {
        functions: Vec<String>,

        /// Keep calls whose arguments call something, that call may matter
        #[serde(default = "default_true")]
        preserve_arguments_side_effects: bool,
    },
}

impl RemoveCalls {
    pub fn functions(&self) -> &[String] {
        match self {
            RemoveCalls::Functions(f) | RemoveCalls::Options { functions: f, .. } => f,
        }
    }

    pub fn preserve_arguments_side_effects(&self) -> bool {
        match self {
            RemoveCalls::Functions(_) => true,
            RemoveCalls::Options {
                preserve_arguments_side_effects,
                ..
            } => *preserve_arguments_side_effects,
        }
    }
}

/// `append_text_comment = { text = "...", location = "start" }`
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AppendTextComment {
    /// Literal comment text, mutually exclusive with `file`
    #[serde(default)]
    pub text: Option<String>,

    /// Read the text from this file instead
    #[serde(default)]
    pub file: Option<PathBuf>,

    #[serde(default = "default_location")]
    pub location: String,
}

fn default_location() -> String {
    "start".to_string()
}

/// Where a rule name lives in coldluau
pub enum RuleStatus {
    /// Works today
    Done,

    /// Designed, lands in this milestone
    Planned(&'static str),

    /// Not a rule here, this is where it lives instead
    Elsewhere(&'static str),
}

/*
Every darklua rule name (32 of them) plus coldluau's own, so a ported
darklua config gets a useful message rather than "unknown key"
*/
pub fn rule_status(name: &str) -> Option<RuleStatus> {
    use RuleStatus::*;

    Some(match name {
        // coldluau rules
        "const_requires"
        | "remove_comments"
        | "append_text_comment"
        | "add_luau_directive"
        | "remove_calls"
        | "use_get_service"
        | "dedupe_requires"
        | "inject_module_path"
        | "freeze_module" => Done,
        // darklua parity lives in ast engine
        "compute_expression"
        | "convert_function_to_assignment"
        | "convert_index_to_field"
        | "convert_local_function_to_assign"
        | "convert_luau_number"
        | "convert_square_root_call"
        | "filter_after_early_return"
        | "group_local_assignment"
        | "make_assignment_local"
        | "remove_assertions"
        | "remove_attribute"
        | "remove_compound_assignment"
        | "remove_continue"
        | "remove_debug_profiling"
        | "remove_floor_division"
        | "remove_function_call_parens"
        | "remove_if_expression"
        | "remove_interpolated_string"
        | "remove_method_call"
        | "remove_method_definition"
        | "remove_nil_declaration"
        | "remove_types"
        | "remove_unused_if_branch"
        | "remove_empty_do"
        | "remove_unused_while" => Done,
        // not rules in coldluau
        "convert_require" => Elsewhere("the [requires] section handles requires"),
        "inject_global_value" => Elsewhere("use [defines] instead (lands in M2)"),
        "remove_spaces" => Elsewhere("use process.generator = \"dense\" (lands in M2)"),
        // still planned, these two need real scope tracking first
        "remove_unused_variable" | "rename_variables" => Planned("M2"),
        _ => return None,
    })
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RequiresConfig {
    #[serde(default)]
    pub target: Target,

    #[serde(default)]
    pub sourcemap: Option<PathBuf>,

    #[serde(default)]
    pub mounts: HashMap<String, String>,

    #[serde(default)]
    pub strict: bool,

    #[serde(default)]
    pub overrides: Option<toml::Value>,

    #[serde(default)]
    pub indexing_style: Option<IndexingStyle>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Target {
    #[default]
    RobloxString,
    Path,
    RobloxInstance,
}

/// How the roblox-instance target indexes children in emitted expressions
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IndexingStyle {
    /// `script.Parent:FindFirstChild("x")`
    #[default]
    #[serde(alias = "find-first-child")]
    FindFirstChild,

    /// `script.Parent:WaitForChild("x")`
    #[serde(alias = "wait-for-child")]
    WaitForChild,

    /// `script.Parent.x` (the dot format)
    #[serde(alias = "property-instance", alias = "property_instance")]
    Property,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RojoConfig {
    #[serde(default)]
    pub project: Option<PathBuf>,
    #[serde(default)]
    pub build_project: Option<PathBuf>,
}

impl Default for ProcessConfig {
    fn default() -> Self {
        Self {
            input: default_input(),
            output: default_output(),
            include: default_include(),
            exclude: Vec::new(),
            generator: default_generator(),
            quotes: QuoteStyle::default(),
            cache: true,
            cache_dir: default_cache_dir(),
        }
    }
}

fn default_input() -> PathBuf {
    "src".into()
}
fn default_output() -> PathBuf {
    "dist".into()
}
fn default_include() -> Vec<String> {
    vec!["**/*.luau".into(), "**/*.lua".into()]
}
fn default_generator() -> String {
    "retain-lines".into()
}
fn default_cache_dir() -> PathBuf {
    ".coldluau".into()
}
fn default_true() -> bool {
    true
}

impl Config {
    pub fn load(path: &Path) -> Result<Self> {
        let text = std::fs::read_to_string(path)
            .with_context(|| format!("failed to read {}", crate::ui::rel(path)))?;

        let config: Config = toml::from_str(&text)
            .with_context(|| format!("invalid config in {}", crate::ui::rel(path)))?;

        config.validate()?;
        Ok(config)
    }

    /// Load `coldluau.toml` from `dir` if present, zero config default otherwise
    pub fn load_or_default(dir: &Path) -> Result<Self> {
        let path = dir.join("coldluau.toml");

        if path.exists() {
            Self::load(&path)
        } else {
            Ok(Self::default())
        }
    }

    fn validate(&self) -> Result<()> {
        let unimplemented: &[(&str, &Option<toml::Value>, &str)] = &[
            ("[defines]", &self.defines, "M2"),
            ("[[extensions]]", &self.extensions, "M3"),
            ("[bundle]", &self.bundle, "M3"),
            ("[minify]", &self.minify, "M4"),
            ("[check]", &self.check, "M3"),
            ("[profile.*]", &self.profile, "M2"),
        ];

        for (name, value, milestone) in unimplemented {
            if value.is_some() {
                bail!("{name} is not implemented yet (lands in {milestone}); remove it for now");
            }
        }

        for name in self.rules.rest.keys() {
            match rule_status(name) {
                Some(RuleStatus::Planned(m)) => bail!(
                    "rule \"{name}\" is not implemented yet, it lands in {m}, remove it for now"
                ),

                Some(RuleStatus::Elsewhere(where_)) => {
                    bail!("\"{name}\" is not a coldluau rule, {where_}")
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
        if self.process.generator != "retain-lines" {
            bail!(
                "generator = \"{}\" is not implemented yet (lands in M2); only \"retain-lines\" works today",
                self.process.generator
            );
        }

        if self.requires.indexing_style.is_some() && self.requires.target != Target::RobloxInstance
        {
            bail!(
                "requires.indexing_style only applies when requires.target = \"roblox-instance\""
            );
        }

        if self.requires.sourcemap.is_some() {
            bail!(
                "requires.sourcemap is not implemented yet (auto-mounts from the Rojo project file cover most cases; sourcemap support lands with M2)"
            );
        }

        if self.requires.overrides.is_some() {
            bail!("[requires.overrides] is not implemented yet (lands in M2)");
        }

        for (name, value) in &self.aliases {
            validate_alias_name(name)?;
            if value.is_empty() {
                bail!("alias \"{name}\" has an empty value");
            }
        }

        Ok(())
    }

    /// Alias map with RFC case insensitive names (lowercased keys)
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

    #[test]
    fn zero_config_defaults() {
        let c: Config = toml::from_str("").unwrap();
        c.validate().unwrap();

        assert_eq!(c.process.input, PathBuf::from("src"));
        assert_eq!(c.requires.target, Target::RobloxString);
    }

    #[test]
    fn coldluau_rules_load_in_both_forms() {
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
        // the list form keeps the safe default
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

    #[test]
    fn unimplemented_sections_error_with_milestone() {
        let c: Config = toml::from_str("[bundle]\nentry = \"src/init.luau\"").unwrap();
        let err = c.validate().unwrap_err().to_string();

        assert!(err.contains("M3"), "{err}");
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
}
