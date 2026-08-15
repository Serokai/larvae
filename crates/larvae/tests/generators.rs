//! The three generators: what `larvae process` writes for each one.

use larvae::config::Config;
use larvae::pipeline;

mod common;
use common::*;

fn processed(config: &str, source: &str) -> String {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();

    write(root, "larvae.toml", config);
    write(root, "src/a.luau", source);

    let cfg = Config::load_or_default(root).unwrap();
    let out = pipeline::run(root, &cfg, true).unwrap();

    assert!(!out.has_errors(), "{:?}", out.diags);

    read(root, "dist/a.luau")
}

const BASE: &str =
    "[process]\ninput = \"src\"\noutput = \"dist\"\n\n[requires]\ntarget = \"path\"\n";

#[test]
fn retain_lines_stays_the_default() {
    let src = "local  x   =  1\n\nreturn   x\n";

    assert_eq!(processed(BASE, src), src);
}

#[test]
fn dense_drops_comments_and_whitespace() {
    let out = processed(
        "[process]\ninput = \"src\"\noutput = \"dist\"\ngenerator = \"dense\"\n\n[requires]\ntarget = \"path\"\n",
        "-- a comment\nlocal  x  =  1\n\nreturn   x  +  2\n",
    );

    assert_eq!(out, "local x=1 return x+2\n");
}

#[test]
fn the_minify_column_span_is_obeyed() {
    let out = processed(
        "[process]\ninput = \"src\"\noutput = \"dist\"\ngenerator = \"dense\"\n\n[minify]\ncolumn_span = 20\n\n[requires]\ntarget = \"path\"\n",
        "local abc = 1\nlocal def = 2\nlocal ghi = 3\nreturn abc + def + ghi\n",
    );

    for line in out.lines() {
        assert!(line.chars().count() <= 20, "{line:?} is over the span");
    }
}

/// `[minify] rename_variables` is the rename rule, on for dense builds only.
#[test]
fn minify_rename_shortens_the_locals() {
    let src = "local function f()\n\tlocal really_long_name = 1\n\treturn really_long_name\nend\n\nreturn f()\n";

    let kept = processed(
        "[process]\ninput = \"src\"\noutput = \"dist\"\ngenerator = \"dense\"\n\n[requires]\ntarget = \"path\"\n",
        src,
    );
    let renamed = processed(
        "[process]\ninput = \"src\"\noutput = \"dist\"\ngenerator = \"dense\"\n\n[minify]\nrename_variables = true\n\n[requires]\ntarget = \"path\"\n",
        src,
    );

    assert!(kept.contains("really_long_name"), "{kept}");
    assert!(!renamed.contains("really_long_name"), "{renamed}");
    assert!(renamed.len() < kept.len());
}

/// The key is inert without the dense generator, like the docs say.
#[test]
fn minify_rename_does_nothing_under_retain_lines() {
    let out = processed(
        "[process]\ninput = \"src\"\noutput = \"dist\"\n\n[minify]\nrename_variables = true\n\n[requires]\ntarget = \"path\"\n",
        "local really_long_name = 1\nreturn really_long_name\n",
    );

    assert!(out.contains("really_long_name"), "{out}");
}

#[test]
fn readable_prints_through_the_formatter() {
    let out = processed(
        "[process]\ninput = \"src\"\noutput = \"dist\"\ngenerator = \"readable\"\n\n[requires]\ntarget = \"path\"\n",
        "local  x=1\nreturn    x\n",
    );

    assert_eq!(out, "local x = 1\nreturn x\n");
}

/// The readable generator prints in the [fmt] style of the project.
#[test]
fn readable_obeys_the_projects_fmt_table() {
    let out = processed(
        "[process]\ninput = \"src\"\noutput = \"dist\"\ngenerator = \"readable\"\n\n[fmt]\nindent_type = \"spaces\"\nindent_width = 2\n\n[requires]\ntarget = \"path\"\n",
        "local function f()\nreturn 1\nend\nreturn f\n",
    );

    assert!(out.contains("\n  return 1\n"), "{out}");
}
