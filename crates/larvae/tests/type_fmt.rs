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

#[test]
fn a_long_type_breaks_at_its_parentheses() {
    assert_eq!(
        fmt_with(NEVERMORE, narrow(40)),
        "export type T = typeof(\n\tsetmetatable(\n\t\t{} :: { a: number },\n\t\t{} :: typeof({ __index = M })\n\t)\n)\n"
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
