/*!
The lints.

Each one gets a case that should fire and at least one that should not, since
a lint that fires on everything is worse than no lint. The near misses are the
half worth reading: they are where the rule stops.
*/

use std::path::Path;

use larvae::diag::Severity;
use larvae::lint::config::Level;
use larvae::lint::{LintConfig, lint};

/// Which lints fired, in source order
fn names(src: &str) -> Vec<String> {
    fired(src, &LintConfig::default())
}

fn fired(src: &str, cfg: &LintConfig) -> Vec<String> {
    lint(Path::new("test.luau"), src, cfg)
        .expect("parses")
        .into_iter()
        .map(|d| {
            let m = d.message;
            let open = m.rfind('(').expect("the lint name is appended");

            m[open + 1..m.len() - 1].to_string()
        })
        .collect()
}

/// Whether one named lint fired
fn fires(name: &str, src: &str) -> bool {
    names(src).iter().any(|n| n == name)
}

fn with(name: &str, level: Level) -> LintConfig {
    let mut cfg = LintConfig::default();
    cfg.rules.insert(name.to_string(), level);

    cfg
}

// --- the registry ----------------------------------------------------------

#[test]
fn every_lint_has_a_distinct_name_and_an_explanation() {
    let mut seen = Vec::new();

    for lint in larvae::lint::registry() {
        assert!(!lint.name().is_empty());
        assert!(!lint.about().is_empty(), "{} has no about", lint.name());
        assert!(
            lint.name().chars().all(|c| c.is_ascii_lowercase() || c == '_'),
            "{} should be snake_case",
            lint.name()
        );
        assert!(!seen.contains(&lint.name()), "{} is registered twice", lint.name());

        seen.push(lint.name());
    }

    assert!(seen.len() >= 15, "expected the full set, found {}", seen.len());
}

#[test]
fn a_lint_can_be_looked_up_by_name() {
    assert!(larvae::lint::find("divide_by_zero").is_some());
    assert!(larvae::lint::find("no_such_lint").is_none());
}

#[test]
fn clean_code_produces_nothing() {
    let src = "local function add(a: number, b: number): number\n\treturn a + b\nend\n\nreturn add\n";

    assert_eq!(names(src), Vec::<String>::new());
}

#[test]
fn a_file_that_does_not_parse_comes_back_as_one_diagnostic() {
    let err = lint(Path::new("test.luau"), "local = = =", &LintConfig::default())
        .expect_err("should not parse");

    assert_eq!(err.severity, Severity::Error);
    assert!(err.message.contains("syntax error"));
}

// --- levels and suppression ------------------------------------------------

#[test]
fn a_lint_set_to_allow_says_nothing() {
    let src = "local x = 1 / 0\n";

    assert!(fires("divide_by_zero", src));
    assert!(
        !fired(src, &with("divide_by_zero", Level::Allow))
            .iter()
            .any(|n| n == "divide_by_zero")
    );
}

#[test]
fn deny_makes_it_an_error_and_warn_leaves_it_a_warning() {
    let src = "local x = 1 / 0\n";

    let at = |level| {
        lint(Path::new("t.luau"), src, &with("divide_by_zero", level))
            .unwrap()
            .into_iter()
            .find(|d| d.message.contains("divide_by_zero"))
            .expect("it fired")
            .severity
    };

    assert_eq!(at(Level::Warn), Severity::Warning);
    assert_eq!(at(Level::Deny), Severity::Error);
}

#[test]
fn a_suppression_comment_silences_one_lint() {
    let src = "-- larvae: allow(divide_by_zero)\nlocal x = 1 / 0\n";

    assert!(!fires("divide_by_zero", src));
}

#[test]
fn a_suppression_for_a_different_lint_does_not_silence_this_one() {
    let src = "-- larvae: allow(shadowing)\nlocal x = 1 / 0\n";

    assert!(fires("divide_by_zero", src));
}

#[test]
fn selenes_suppression_spelling_works_too() {
    let src = "-- selene: allow(divide_by_zero)\nlocal x = 1 / 0\n";

    assert!(!fires("divide_by_zero", src));
}

// --- correctness -----------------------------------------------------------

#[test]
fn almost_swapped_catches_the_two_line_swap() {
    assert!(fires("almost_swapped", "a = b\nb = a\n"));
    assert!(fires("almost_swapped", "t.x = t.y\nt.y = t.x\n"));
}

#[test]
fn a_real_swap_is_not_reported() {
    assert!(!fires("almost_swapped", "a, b = b, a\n"));
    assert!(!fires("almost_swapped", "local tmp = a\na = b\nb = tmp\n"));
}

/// Two unrelated assignments that happen to be adjacent
#[test]
fn assignments_that_are_not_a_swap_are_left_alone() {
    assert!(!fires("almost_swapped", "a = b\nc = d\n"));
    assert!(!fires("almost_swapped", "a = a\na = a\n"));
}

#[test]
fn compare_nan_catches_the_zero_over_zero_idiom() {
    assert!(fires("compare_nan", "if x == 0/0 then end\n"));
    assert!(fires("compare_nan", "if 0/0 ~= x then end\n"));
}

/// The correct nan test, which must not be reported
#[test]
fn comparing_a_value_to_itself_is_not_reported() {
    assert!(!fires("compare_nan", "if x ~= x then end\n"));
}

#[test]
fn constant_table_comparison_catches_comparing_to_a_literal() {
    assert!(fires("constant_table_comparison", "if t == {} then end\n"));
    assert!(fires("constant_table_comparison", "if t ~= { a = 1 } then end\n"));
}

#[test]
fn comparing_two_named_tables_is_a_real_question() {
    assert!(!fires("constant_table_comparison", "if a == b then end\n"));
}

#[test]
fn divide_by_zero_catches_every_dividing_operator() {
    assert!(fires("divide_by_zero", "local x = n / 0\n"));
    assert!(fires("divide_by_zero", "local x = n // 0\n"));
    assert!(fires("divide_by_zero", "local x = n % 0\n"));
}

#[test]
fn dividing_by_something_that_might_be_zero_is_not_reported() {
    assert!(!fires("divide_by_zero", "local x = n / d\n"));
}

/// `0/0` is how nan is written, and compare_nan owns that case
#[test]
fn nan_is_not_also_reported_as_a_division() {
    assert!(!fires("divide_by_zero", "local x = 0/0\n"));
}

#[test]
fn duplicate_keys_catches_a_repeated_name_and_a_repeated_literal() {
    assert!(fires("duplicate_keys", "local t = { a = 1, a = 2 }\n"));
    assert!(fires("duplicate_keys", "local t = { [1] = 'x', [1] = 'y' }\n"));
    assert!(fires("duplicate_keys", "local t = { ['k'] = 1, ['k'] = 2 }\n"));
}

#[test]
fn distinct_keys_are_fine_and_a_computed_key_is_not_guessed_at() {
    assert!(!fires("duplicate_keys", "local t = { a = 1, b = 2 }\n"));
    assert!(!fires("duplicate_keys", "local t = { [i] = 1, [j] = 2 }\n"));
    assert!(!fires("duplicate_keys", "local t = { 1, 2, 3 }\n"));
}

#[test]
fn ifs_same_cond_catches_a_branch_that_can_never_run() {
    assert!(fires("ifs_same_cond", "if a then x() elseif a then y() end\n"));
}

#[test]
fn different_conditions_are_fine() {
    assert!(!fires("ifs_same_cond", "if a then x() elseif b then y() end\n"));
}

#[test]
fn if_same_then_else_catches_two_identical_branches() {
    assert!(fires("if_same_then_else", "if a then\n\tx()\nelse\n\tx()\nend\n"));
}

#[test]
fn branches_that_differ_are_fine() {
    assert!(!fires("if_same_then_else", "if a then\n\tx()\nelse\n\ty()\nend\n"));
}

#[test]
fn suspicious_reverse_loop_catches_a_countdown_without_a_step() {
    assert!(fires("suspicious_reverse_loop", "for i = 10, 1 do print(i) end\n"));
}

#[test]
fn a_countdown_with_a_negative_step_is_correct() {
    assert!(!fires("suspicious_reverse_loop", "for i = 10, 1, -1 do print(i) end\n"));
    assert!(!fires("suspicious_reverse_loop", "for i = 1, 10 do print(i) end\n"));
}

/// A limit that is not a literal could be anything
#[test]
fn a_loop_over_computed_bounds_is_not_guessed_at() {
    assert!(!fires("suspicious_reverse_loop", "for i = n, 1 do print(i) end\n"));
    assert!(!fires("suspicious_reverse_loop", "for i = 10, #t do print(i) end\n"));
}

#[test]
fn type_check_inside_call_catches_the_misplaced_parenthesis() {
    assert!(fires("type_check_inside_call", "if type(x == 'number') then end\n"));
    assert!(fires("type_check_inside_call", "if typeof(x == 'Vector3') then end\n"));
}

#[test]
fn the_correct_form_is_not_reported() {
    assert!(!fires("type_check_inside_call", "if type(x) == 'number' then end\n"));
}

#[test]
fn unbalanced_assignments_catches_both_directions() {
    assert!(fires("unbalanced_assignments", "local a, b = 1\n"));
    assert!(fires("unbalanced_assignments", "a, b = 1, 2, 3\n"));
}

#[test]
fn a_matched_assignment_is_fine() {
    assert!(!fires("unbalanced_assignments", "local a, b = 1, 2\n"));
}

/// Declaring names to fill in later is normal, not an imbalance
#[test]
fn a_declaration_with_no_values_is_not_reported() {
    assert!(!fires("unbalanced_assignments", "local a, b\n"));
}

/// A call can return any number of values, so the counts need not match
#[test]
fn a_call_or_vararg_in_last_position_excuses_the_count() {
    assert!(!fires("unbalanced_assignments", "local a, b = f()\n"));
    assert!(!fires("unbalanced_assignments", "local a, b = ...\n"));
    assert!(!fires("unbalanced_assignments", "local a, b, c = 1, f()\n"));
}

// --- style -----------------------------------------------------------------

#[test]
fn empty_if_catches_an_empty_branch() {
    assert!(fires("empty_if", "if a then end\n"));
    assert!(fires("empty_if", "if a then x() else end\n"));
}

/// A branch holding a comment is deliberate, the comment is the content
#[test]
fn a_branch_with_only_a_comment_is_left_alone() {
    assert!(!fires("empty_if", "if a then\n\t-- nothing to do yet\nend\n"));
}

#[test]
fn empty_loop_catches_every_loop_form() {
    assert!(fires("empty_loop", "while true do end\n"));
    assert!(fires("empty_loop", "for i = 1, 10 do end\n"));
    assert!(fires("empty_loop", "for k in pairs(t) do end\n"));
    assert!(fires("empty_loop", "repeat until done\n"));
}

#[test]
fn a_loop_with_a_body_is_fine() {
    assert!(!fires("empty_loop", "while true do work() end\n"));
}

#[test]
fn mixed_table_catches_both_halves_in_one_table() {
    assert!(fires("mixed_table", "local t = { 1, 2, a = 3 }\n"));
}

#[test]
fn a_table_that_is_only_one_shape_is_fine() {
    assert!(!fires("mixed_table", "local t = { 1, 2, 3 }\n"));
    assert!(!fires("mixed_table", "local t = { a = 1, b = 2 }\n"));
}

#[test]
fn parenthese_conditions_catches_the_habit() {
    assert!(fires("parenthese_conditions", "if (a) then end\n"));
    assert!(fires("parenthese_conditions", "while (a) do x() end\n"));
}

#[test]
fn parentheses_that_group_something_are_left_alone() {
    assert!(!fires("parenthese_conditions", "if (a or b) and c then end\n"));
}

#[test]
fn multiple_statements_is_off_until_a_project_asks() {
    let src = "local a = 1 local b = 2\n";

    assert!(!fires("multiple_statements", src));
    assert!(
        fired(src, &with("multiple_statements", Level::Warn))
            .iter()
            .any(|n| n == "multiple_statements")
    );
}

/// The idiom this lint must not report, even when it is on
#[test]
fn a_one_line_guard_is_not_two_statements() {
    let cfg = with("multiple_statements", Level::Warn);

    assert!(
        !fired("if x then return end\n", &cfg)
            .iter()
            .any(|n| n == "multiple_statements"),
        "the return is in its own block"
    );
}

// --- names -----------------------------------------------------------------

#[test]
fn unused_variable_catches_a_local_nothing_reads() {
    assert!(fires("unused_variable", "local x = 1\n"));
    assert!(fires("unused_variable", "local function helper() end\n"));
}

#[test]
fn a_local_that_is_read_is_fine() {
    assert!(!fires("unused_variable", "local x = 1\nprint(x)\n"));
}

/// The convention for a name deliberately not used
#[test]
fn an_underscore_name_is_exempt() {
    assert!(!fires("unused_variable", "local _ = f()\n"));
    assert!(!fires("unused_variable", "local _unused = 1\n"));
}

/// A signature is the caller's shape, and `for k, v` where only k is wanted is normal
#[test]
fn parameters_and_loop_variables_are_not_reported_by_default() {
    assert!(!fires("unused_variable", "local function f(a, b)\n\treturn a\nend\nprint(f)\n"));
    assert!(!fires("unused_variable", "for k, v in pairs(t) do print(k) end\n"));
}

#[test]
fn parameters_can_be_asked_for() {
    let mut cfg = LintConfig::default();
    cfg.options.insert(
        "unused_variable".into(),
        toml::from_str::<toml::Value>("parameters = true").unwrap(),
    );

    let src = "local function f(a, b)\n\treturn a\nend\nprint(f)\n";

    assert!(fired(src, &cfg).iter().any(|n| n == "unused_variable"));
}

/// A value computed and thrown away reads as a bug, not as tidying
#[test]
fn a_variable_written_but_never_read_is_reported_differently() {
    let out = lint(
        Path::new("t.luau"),
        "local x = 1\nx = 2\n",
        &LintConfig::default(),
    )
    .unwrap();

    assert!(
        out.iter().any(|d| d.message.contains("assigned but never read")),
        "{:?}",
        out.iter().map(|d| &d.message).collect::<Vec<_>>()
    );
}

#[test]
fn undefined_variable_catches_a_name_nothing_declares() {
    assert!(fires("undefined_variable", "print(notDeclaredAnywhere)\n"));
}

#[test]
fn a_standard_global_is_defined() {
    assert!(!fires("undefined_variable", "print(pairs)\n"));
    assert!(!fires("undefined_variable", "local s = game:GetService('Players')\nprint(s)\n"));
}

#[test]
fn a_project_can_add_its_own_globals() {
    let mut cfg = LintConfig::default();
    cfg.globals.push("MyFramework".to_string());

    assert!(
        !fired("print(MyFramework)\n", &cfg)
            .iter()
            .any(|n| n == "undefined_variable")
    );
}

/// A Roblox global should not be defined when the project is plain Luau
#[test]
fn the_std_setting_decides_which_globals_exist() {
    let mut cfg = LintConfig::default();
    cfg.std = larvae::lint::config::StdLib::Luau;

    assert!(
        fired("print(game)\n", &cfg)
            .iter()
            .any(|n| n == "undefined_variable")
    );
}

/// It is nil at runtime, which is not a matter of taste
#[test]
fn undefined_variable_is_an_error_rather_than_a_warning() {
    let out = lint(
        Path::new("t.luau"),
        "print(notDeclaredAnywhere)\n",
        &LintConfig::default(),
    )
    .unwrap();

    assert_eq!(out[0].severity, Severity::Error);
}

#[test]
fn unscoped_variables_catches_the_missing_local() {
    assert!(fires("unscoped_variables", "counter = 1\n"));
}

#[test]
fn a_local_assignment_is_fine() {
    assert!(!fires("unscoped_variables", "local counter = 0\ncounter = 1\nprint(counter)\n"));
}

#[test]
fn shadowing_catches_a_hidden_name() {
    assert!(fires("shadowing", "local x = 1\nprint(x)\ndo\n\tlocal x = 2\n\tprint(x)\nend\n"));
}

/// Re-deriving what you hide is the deliberate form
#[test]
fn a_binding_that_reads_what_it_hides_is_not_reported() {
    assert!(!fires("shadowing", "local x = 1\ndo\n\tlocal x = x + 1\n\tprint(x)\nend\n"));
}

#[test]
fn a_sibling_scope_is_not_shadowing() {
    assert!(!fires("shadowing", "do\n\tlocal x = 1\n\tprint(x)\nend\ndo\n\tlocal x = 2\n\tprint(x)\nend\n"));
}

#[test]
fn global_usage_catches_reaching_into_the_shared_table() {
    assert!(fires("global_usage", "_G.thing = 1\n"));
    assert!(fires("global_usage", "print(_G.thing)\n"));
}

/// Somebody's own table named _G is theirs
#[test]
fn a_local_named_underscore_g_is_not_the_shared_one() {
    assert!(!fires("global_usage", "local _G = {}\nprint(_G.thing)\n"));
}

// --- beyond selene ---------------------------------------------------------

#[test]
fn unreachable_code_catches_statements_after_an_exit() {
    assert!(fires("unreachable_code", "while true do\n\tbreak\n\tprint('never')\nend\n"));
    assert!(fires("unreachable_code", "for i = 1, 3 do\n\tcontinue\n\tprint('never')\nend\n"));
}

#[test]
fn code_before_the_exit_is_fine() {
    assert!(!fires("unreachable_code", "while true do\n\tprint('once')\n\tbreak\nend\n"));
}

/// An exit inside a branch leaves the enclosing block reachable
#[test]
fn a_conditional_exit_does_not_kill_what_follows() {
    assert!(!fires(
        "unreachable_code",
        "while true do\n\tif done then\n\t\tbreak\n\tend\n\tprint('still runs')\nend\n"
    ));
}

#[test]
fn self_assignment_catches_a_line_that_does_nothing() {
    assert!(fires("self_assignment", "local x = 1\nx = x\nprint(x)\n"));
    assert!(fires("self_assignment", "t.k = t.k\n"));
}

#[test]
fn assigning_something_different_is_fine() {
    assert!(!fires("self_assignment", "local x = 1\nx = y\nprint(x)\n"));
    assert!(!fires("self_assignment", "t.a = t.b\n"));
}

/// A compound operator does something, and a computed index may not be the same element
#[test]
fn compound_and_computed_forms_are_left_alone() {
    assert!(!fires("self_assignment", "local x = 1\nx += x\nprint(x)\n"));
    assert!(!fires("self_assignment", "t[i] = t[i]\n"));
}

#[test]
fn string_concat_in_loop_catches_the_quadratic_build() {
    assert!(fires(
        "string_concat_in_loop",
        "local s = ''\nfor i = 1, 10 do\n\ts = s .. i\nend\nprint(s)\n"
    ));
    assert!(fires(
        "string_concat_in_loop",
        "local s = ''\nwhile go do\n\ts = s .. 'x'\nend\nprint(s)\n"
    ));
}

#[test]
fn assigning_a_fresh_string_each_round_is_not_accumulating() {
    assert!(!fires(
        "string_concat_in_loop",
        "local s = ''\nfor i = 1, 10 do\n\ts = 'a' .. i\nend\nprint(s)\n"
    ));
}

#[test]
fn table_concat_after_the_loop_is_the_recommended_shape_and_is_quiet() {
    assert!(!fires(
        "string_concat_in_loop",
        "local parts = {}\nfor i = 1, 10 do\n\ttable.insert(parts, i)\nend\nprint(table.concat(parts))\n"
    ));
}

#[test]
fn loop_invariant_call_catches_a_service_lookup_per_iteration() {
    assert!(fires(
        "loop_invariant_call",
        "for i = 1, 10 do\n\tlocal p = game:GetService('Players')\n\tprint(p)\nend\n"
    ));
    assert!(fires(
        "loop_invariant_call",
        "for i = 1, 10 do\n\tlocal m = require('@pkg/thing')\n\tprint(m)\nend\n"
    ));
}

#[test]
fn a_lookup_hoisted_above_the_loop_is_quiet() {
    assert!(!fires(
        "loop_invariant_call",
        "local p = game:GetService('Players')\nfor i = 1, 10 do\n\tprint(p)\nend\n"
    ));
}

/// A call whose argument varies is not invariant
#[test]
fn a_call_with_a_computed_argument_is_not_reported() {
    assert!(!fires(
        "loop_invariant_call",
        "for i = 1, 10 do\n\tlocal m = require(names[i])\n\tprint(m)\nend\n"
    ));
}

/// A function body does not run once per iteration
#[test]
fn a_call_inside_a_callback_is_not_this_loops_work() {
    assert!(!fires(
        "loop_invariant_call",
        "for i = 1, 10 do\n\tdefer(function()\n\t\tprint(game:GetService('Players'))\n\tend)\nend\n"
    ));
}
