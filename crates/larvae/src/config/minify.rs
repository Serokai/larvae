/*!
`[minify]`, the tuning table for `generator = "dense"`.

The dense generator is the minifier: it re-emits the tokens of the output
with the least whitespace that lexes the same. This table tunes that
emission. With another generator the table is inert configuration, the same
as a `[fmt]` table next to a stylua.toml.
*/

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "snake_case")]
pub struct MinifyConfig {
    /*
    The column where the emitter breaks the line.

    A minified file on one line is hostile to every tool that reports
    line numbers, and a crash report with `line 1` says nothing. A break
    near a column keeps the file dense and keeps positions meaningful. A
    token longer than the span, ex: a long string, stays whole.
    */
    #[serde(default = "default_column_span")]
    pub column_span: usize,

    /*
    Give every local a short name while minifying.

    Off by default, like every rule. The key is a convenience: it turns on
    the `rename_variables` rule for a dense build without editing `[rules]`,
    so one profile can hold the whole minify story.
    */
    #[serde(default)]
    pub rename_variables: bool,
}

fn default_column_span() -> usize {
    120
}

impl Default for MinifyConfig {
    fn default() -> Self {
        toml::from_str("").expect("every field has a default")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_defaults_break_lines_and_rename_nothing() {
        let c = MinifyConfig::default();

        assert_eq!(c.column_span, 120);
        assert!(!c.rename_variables);
    }

    #[test]
    fn an_unknown_key_is_refused_like_everywhere_else() {
        assert!(toml::from_str::<MinifyConfig>("colum_span = 80").is_err());
    }
}
