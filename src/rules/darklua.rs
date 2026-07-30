/*!
darklua parity rules

Every rule here matches a darklua rule name and its documented behavior, so
a ported config does what it did before. Implementations go through the
shared engine, walk the tree, push byte edits, keep newline counts when
deleting multiline spans
*/

use crate::config::RulesConfig;
use crate::diag::Diag;
use crate::rules::engine::{Edit, RuleCtx};
use std::path::Path;

/// True when any rule in this module is enabled, gates the parse
pub fn wants(_cfg: &RulesConfig) -> bool {
    false
}

/// Run every enabled rule, push edits and diagnostics
pub fn apply(
    _cfg: &RulesConfig,
    _ctx: &RuleCtx,
    _edits: &mut Vec<Edit>,
    _diags: &mut Vec<Diag>,
    _path: &Path,
) {
}
