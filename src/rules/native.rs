/*!
coldluau's own rules

Rules with no darklua equivalent, they lean on what only coldluau knows,
resolved require forms and the datamodel path of every file. Same contract
as the parity rules, walk the tree, push byte edits, keep newline counts
when deleting multiline spans
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
