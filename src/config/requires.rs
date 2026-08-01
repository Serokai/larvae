//! The [requires] and [rojo] tables, what a rewritten require turns into

use std::collections::HashMap;
use std::path::PathBuf;

use serde::Deserialize;

use super::process::default_true;

#[derive(Debug, Deserialize)]
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

    /*
    Read require(script.Parent.Foo) style requires and rewrite them to the
    configured target. On by default because it is the whole point of moving
    an existing codebase over, off is the escape hatch for anyone who wants
    their instance requires left exactly as written
    */
    #[serde(default = "default_true")]
    pub instance_input: bool,

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

impl Default for RequiresConfig {
    fn default() -> Self {
        Self {
            target: Target::default(),
            sourcemap: None,
            mounts: HashMap::new(),
            strict: false,
            instance_input: true,
            overrides: None,
            indexing_style: None,
        }
    }
}
