//! The root `exclude` and `include`, which the process walk inherits.

use larvae::config::Config;
use larvae::pipeline;

mod common;
use common::*;

/*
The root `exclude` removes a file from every command. So the process walk
drops it fully: the file is not transformed and not copied. The `[process]`
lists keep their own meaning beside it.
*/
#[test]
fn the_root_exclude_removes_a_file_from_the_process_walk() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();

    write(
        root,
        "larvae.toml",
        "exclude = [\"src/gen\"]\n\n[process]\ninput = \"src\"\noutput = \"dist\"\n\n[requires]\ntarget = \"path\"\n",
    );
    write(root, "src/a.luau", "return 1\n");
    write(root, "src/gen/out.luau", "return 2\n");
    write(root, "src/gen/data.txt", "raw\n");

    let config = Config::load_or_default(root).unwrap();
    let out = pipeline::run(root, &config, true).unwrap();

    assert!(!out.has_errors(), "{:?}", out.diags);
    assert!(root.join("dist/a.luau").exists());
    assert!(!root.join("dist/gen/out.luau").exists(), "not transformed");
    assert!(!root.join("dist/gen/data.txt").exists(), "not copied");
}

/// The root `include` cancels the root `exclude`, and only that.
#[test]
fn the_root_include_brings_a_file_back_into_the_walk() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();

    write(
        root,
        "larvae.toml",
        "exclude = [\"src/gen\"]\ninclude = [\"src/gen/keep.luau\"]\n\n[process]\ninput = \"src\"\noutput = \"dist\"\n\n[requires]\ntarget = \"path\"\n",
    );
    write(root, "src/gen/keep.luau", "return 1\n");
    write(root, "src/gen/out.luau", "return 2\n");

    let config = Config::load_or_default(root).unwrap();
    let out = pipeline::run(root, &config, true).unwrap();

    assert!(!out.has_errors(), "{:?}", out.diags);
    assert!(root.join("dist/gen/keep.luau").exists());
    assert!(!root.join("dist/gen/out.luau").exists());
}
