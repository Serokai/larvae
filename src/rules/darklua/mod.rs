/*!
darklua parity rules

Every rule here matches a darklua rule name and its documented behavior, so
a ported config does what it did before. Implementations go through the
shared engine, walk the tree, push byte edits, keep newline counts when
deleting multiline spans

A rule that cannot prove a transform safe from the tree alone skips that
instance in silence, conservative beats wrong, and rules never see each
other's output so anything darklua reaches by chaining is done here in one
pass instead
*/

mod assign;
mod calls;
mod eval;
mod exprs;
mod flow;
mod fold;
mod interp;
mod methods;
mod support;
mod types;

use crate::config::RulesConfig;
use crate::diag::Diag;
use crate::rules::engine::{Edit, RuleCtx};
use std::path::Path;

/// True when any rule in this module is enabled, gates the parse
pub fn wants(cfg: &RulesConfig) -> bool {
    cfg.remove_method_definition
        || cfg.remove_compound_assignment
        || cfg.remove_floor_division
        || cfg.remove_if_expression
        || cfg.remove_method_call
        || cfg.convert_index_to_field
        || cfg.convert_function_to_assignment
        || cfg.convert_luau_number
        || cfg.make_assignment_local
        || cfg.remove_types
        || cfg.remove_function_call_parens
        || cfg.filter_after_early_return
        || cfg.remove_continue
        || cfg.compute_expression
        || cfg.remove_unused_if_branch
        || cfg.remove_unused_while
        || cfg.remove_empty_do
        || cfg.remove_nil_declaration
        || cfg.group_local_assignment
        || cfg.convert_local_function_to_assign
        || cfg.convert_square_root_call
        || cfg.remove_attribute.as_ref().is_some_and(|r| r.enabled())
        || cfg
            .remove_interpolated_string
            .as_ref()
            .is_some_and(|r| r.enabled())
        || cfg.remove_assertions.as_ref().is_some_and(|r| r.enabled())
        || cfg
            .remove_debug_profiling
            .as_ref()
            .is_some_and(|r| r.enabled())
}

/// Run every enabled rule, push edits and diagnostics
pub fn apply(
    cfg: &RulesConfig,
    ctx: &RuleCtx,
    edits: &mut Vec<Edit>,
    _diags: &mut Vec<Diag>,
    _path: &Path,
) {
    /*
    convert_function_to_assignment already rewrites a method definition
    head and inserts the self parameter, letting remove_method_definition
    add a second one would emit `self, self`, so the broader rule wins
    */
    if cfg.remove_method_definition && !cfg.convert_function_to_assignment {
        methods::remove_method_definition(ctx, edits);
    }

    if cfg.convert_function_to_assignment {
        methods::convert_function_to_assignment(ctx, edits);
    }

    if cfg.convert_local_function_to_assign {
        methods::convert_local_function_to_assign(ctx, edits);
    }

    if cfg.remove_method_call {
        methods::remove_method_call(ctx, edits);
    }

    if cfg.remove_compound_assignment {
        assign::remove_compound_assignment(ctx, edits, cfg.remove_floor_division);
    }

    if cfg.remove_floor_division {
        assign::remove_floor_division(ctx, edits);
    }

    if cfg.make_assignment_local {
        assign::make_assignment_local(ctx, edits);
    }

    if cfg.remove_nil_declaration {
        assign::remove_nil_declaration(ctx, edits);
    }

    if cfg.group_local_assignment {
        assign::group_local_assignment(ctx, edits);
    }

    if cfg.remove_if_expression {
        exprs::remove_if_expression(ctx, edits);
    }

    if cfg.convert_index_to_field {
        exprs::convert_index_to_field(ctx, edits);
    }

    if cfg.convert_luau_number {
        exprs::convert_luau_number(ctx, edits);
    }

    if cfg.remove_function_call_parens {
        exprs::remove_function_call_parens(ctx, edits);
    }

    if cfg.convert_square_root_call {
        exprs::convert_square_root_call(ctx, edits);
    }

    if cfg.remove_types {
        types::remove_types(ctx, edits);
    }

    if let Some(r) = &cfg.remove_attribute
        && r.enabled()
    {
        types::remove_attribute(ctx, edits, r.patterns());
    }

    if let Some(r) = &cfg.remove_interpolated_string
        && r.enabled()
    {
        interp::remove_interpolated_string(ctx, edits, r.strategy());
    }

    if cfg.filter_after_early_return {
        flow::filter_after_early_return(ctx, edits);
    }

    if cfg.remove_continue {
        flow::remove_continue(ctx, edits);
    }

    if cfg.remove_unused_while {
        flow::remove_unused_while(ctx, edits);
    }

    if cfg.remove_unused_if_branch {
        flow::remove_unused_if_branch(ctx, edits);
    }

    if cfg.remove_empty_do {
        flow::remove_empty_do(ctx, edits);
    }

    if let Some(r) = &cfg.remove_assertions
        && r.enabled()
    {
        calls::remove_assertions(ctx, edits, r.preserve());
    }

    if let Some(r) = &cfg.remove_debug_profiling
        && r.enabled()
    {
        calls::remove_debug_profiling(ctx, edits, r.preserve());
    }

    if cfg.compute_expression {
        fold::compute_expression(ctx, edits);
    }
}

#[cfg(test)]
pub(crate) mod testing {
    /*
    Rule tests all want the same thing, parse a snippet, run one rule, splice
    the edits the way the pipeline does, compare the text
    */
    use crate::rules::engine::{Edit, RuleCtx};
    use crate::syntax::{lexer, parser};

    pub fn run(src: &str, rule: impl Fn(&RuleCtx, &mut Vec<Edit>)) -> String {
        let lexed = lexer::lex(src).expect("lexes");
        let chunk = parser::parse(src, &lexed.toks).expect("parses");

        let ctx = RuleCtx {
            src,
            toks: &lexed.toks,
            chunk: &chunk,
            comments: &lexed.comments,
            require_forms: &[],
            dm_path: None,
            quote: '"',
        };

        let mut edits: Vec<Edit> = Vec::new();
        rule(&ctx, &mut edits);

        splice(src, &mut edits)
    }

    /// Same ordering and overlap policy as the pipeline splice
    pub fn splice(src: &str, edits: &mut [Edit]) -> String {
        edits.sort_by_key(|e| (e.0, e.1));

        let mut out = String::new();
        let mut cursor = 0usize;

        for (start, end, new) in edits.iter() {
            if (*start as usize) < cursor {
                continue;
            }

            out.push_str(&src[cursor..*start as usize]);
            out.push_str(new);
            cursor = *end as usize;
        }

        out.push_str(&src[cursor..]);

        out
    }

    /// Every rule must keep the line count stable for retain-lines output
    pub fn assert_lines_kept(before: &str, after: &str) {
        assert_eq!(
            before.bytes().filter(|&b| b == b'\n').count(),
            after.bytes().filter(|&b| b == b'\n').count(),
            "line count drifted\nbefore:\n{before}\nafter:\n{after}"
        );
    }
}
