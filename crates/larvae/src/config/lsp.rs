/*!
`[lsp]`, how the editor server behaves.

The plan for the server is a claim-only default once the Luau analyzer
lands: answer for the files that worms claim, and coexist with luau-lsp on
the rest. Today's server lints and formats plain Luau too, and projects
rely on that, so `claim_only` defaults to off until the analyzer era, and
the flip will be a stated breaking change.
*/

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "snake_case")]
pub struct LspConfig {
    /// Off answers every request with nothing, so another server owns the files
    #[serde(default = "on")]
    pub enabled: bool,

    /// On serves only the files that a worm claims; off serves every Luau file
    #[serde(default)]
    pub claim_only: bool,

    /// What the completion list offers, and how it writes what it inserts
    #[serde(default)]
    pub completion: CompletionConfig,
}

/*
`[lsp.completion]`, which mirrors luau-lsp's `completion.*` settings.

The names match luau-lsp's, with larvae's snake_case spelling, so a user who
moves between the two servers keeps the setting they already know. The editor
extension exposes the same ids under `larvae-lsp.`, and this table is the
project side of them. Where both speak, the project wins.
*/
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "snake_case")]
pub struct CompletionConfig {
    #[serde(default)]
    pub imports: ImportsConfig,
}

/// `[lsp.completion.imports]`, the auto-import settings
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "snake_case")]
pub struct ImportsConfig {
    /*
    Whether an auto-import writes `const` or `local`.

    On by default, which is a deliberate departure from luau-lsp. That server
    defaults it off because Luau had no `const` when the setting was written.
    Larvae's platform has the keyword, and an auto-import is the clearest case
    for it: the line binds a service or a module and nothing reassigns it.

    A project that has not adopted `const` sets this off, and the completion
    writes `local` instead. The setting is its own thing and does not read
    `[fmt] require_binding`, because that option governs a `require` binding
    and a `game:GetService` line is not one. To tie them would make the
    formatter decide what the editor types.
    */
    #[serde(default = "on")]
    pub use_const: bool,
}

impl Default for ImportsConfig {
    fn default() -> Self {
        toml::from_str("").expect("every field has a default")
    }
}

impl ImportsConfig {
    /// The keyword an auto-import writes
    pub fn keyword(&self) -> &'static str {
        match self.use_const {
            true => "const",

            false => "local",
        }
    }
}

fn on() -> bool {
    true
}

impl Default for LspConfig {
    fn default() -> Self {
        toml::from_str("").expect("every field has a default")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_server_is_on_and_serves_everything_by_default() {
        let c = LspConfig::default();

        assert!(c.enabled);
        assert!(!c.claim_only);
    }

    #[test]
    fn an_unknown_key_is_refused_like_everywhere_else() {
        assert!(toml::from_str::<LspConfig>("clam_only = true").is_err());
    }
}
