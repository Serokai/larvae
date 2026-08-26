use larvae::fmt::config::{TableTypes, TypeExpansion};
use larvae::fmt::{FmtConfig, format};

fn fmt(src: &str) -> String {
    format(src, &FmtConfig::default()).expect("formats")
}

fn narrow(width: usize) -> FmtConfig {
    FmtConfig {
        column_width: width,
        ..Default::default()
    }
}

fn fmt_with(src: &str, cfg: FmtConfig) -> String {
    format(src, &cfg).expect("formats")
}

const NEVERMORE: &str =
    "export type T = typeof(setmetatable({} :: { a: number }, {} :: typeof({ __index = M })))\n";

#[test]
fn a_type_assertion_keeps_its_spaces() {
    assert_eq!(fmt(NEVERMORE), NEVERMORE);
}

#[test]
fn a_type_that_fits_stays_on_one_line() {
    assert_eq!(fmt_with(NEVERMORE, narrow(120)), NEVERMORE);
}

/// A wrapper over one call breaks at that call, so `typeof(` stays with the
/// `setmetatable(` it holds and the two closing brackets stay together.
#[test]
fn a_long_type_breaks_at_the_call_it_wraps() {
    assert_eq!(
        fmt_with(NEVERMORE, narrow(40)),
        "export type T = typeof(setmetatable(\n\t{} :: { a: number },\n\t{} :: typeof({ __index = M })\n))\n"
    );
}

/// A region of several parts still opens one per line.
#[test]
fn a_long_function_type_breaks_one_parameter_per_line() {
    let src = "export type F = (alphaalpha: string, betabeta: number, gammagamma: boolean) -> ()\n";

    assert_eq!(
        fmt_with(src, narrow(40)),
        "export type F = (\n\talphaalpha: string,\n\tbetabeta: number,\n\tgammagamma: boolean\n) -> ()\n"
    );
}

/// A broken parenthesised type must not gain a trailing comma: `typeof(x,)`
/// does not parse.
#[test]
fn a_broken_type_has_no_trailing_comma() {
    let out = fmt_with(NEVERMORE, narrow(40));

    assert!(!out.contains(",\n)"), "trailing comma before a close paren");
    assert!(
        !out.contains(",\n\t)"),
        "trailing comma before a close paren"
    );
}

/// Still open: unions need a break point on the top level `|` and `&`.
#[test]
fn a_long_union_stays_on_one_line_for_now() {
    let src = "export type U = \"alphaalpha\" | \"betabeta\" | \"gammagamma\" | \"deltadelta\"\n";

    assert_eq!(fmt_with(src, narrow(40)), src);
}

fn expand(expand: TypeExpansion) -> FmtConfig {
    FmtConfig {
        table_types: TableTypes {
            expand,
            ..Default::default()
        },
        ..Default::default()
    }
}

#[test]
fn when_needed_collapses_an_author_wrapped_table() {
    assert_eq!(
        fmt_with(
            "type T = {\n\tx: number,\n}\n",
            expand(TypeExpansion::WhenNeeded)
        ),
        "type T = { x: number }\n"
    );
}

#[test]
fn always_opens_a_table_that_fits() {
    assert_eq!(
        fmt_with("type T = { x: number }\n", expand(TypeExpansion::Always)),
        "type T = {\n\tx: number,\n}\n"
    );
}

#[test]
fn always_leaves_an_empty_table_flat() {
    assert_eq!(
        fmt_with("type T = {}\n", expand(TypeExpansion::Always)),
        "type T = {}\n"
    );
}

#[test]
fn preserve_keeps_an_author_wrapped_table_open() {
    let src = "type T = {\n\tx: number,\n}\n";

    assert_eq!(fmt_with(src, expand(TypeExpansion::Preserve)), src);
}

#[test]
fn preserve_keeps_an_author_flat_table_flat() {
    let src = "type T = { x: number }\n";

    assert_eq!(fmt_with(src, expand(TypeExpansion::Preserve)), src);
}

#[test]
fn preserve_holds_the_nevermore_shape() {
    let src = "export type S = typeof(setmetatable(\n\t{} :: {\n\t\t_serviceBag: ServiceBag.ServiceBag,\n\t},\n\t{} :: typeof({ __index = S })\n))\n";

    assert_eq!(fmt_with(src, expand(TypeExpansion::Preserve)), src);
}
