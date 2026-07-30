//! End-to-end pipeline tests against a realistic fixture project

use std::fs;
use std::path::Path;

use coldluau::config::Config;
use coldluau::diag::Severity;
use coldluau::pipeline;

fn write(root: &Path, rel: &str, content: &str) {
    let p = root.join(rel);
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, content).unwrap();
}

fn read(root: &Path, rel: &str) -> String {
    fs::read_to_string(root.join(rel)).unwrap()
}

/// A Rojo shaped project, shared + server code, packages outside src
fn fixture() -> tempfile::TempDir {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();

    write(
        root,
        "default.project.json",
        r#"{
            "name": "fixture",
            "tree": {
                "$className": "DataModel",
                "ReplicatedStorage": {
                    "shared": { "$path": "src/shared" },
                    "Packages": { "$path": "Packages" }
                },
                "ServerScriptService": { "$path": "src/server" }
            }
        }"#,
    );
    write(
        root,
        "coldluau.toml",
        r#"
            [aliases]
            pkg = "@game/ReplicatedStorage/Packages"
        "#,
    );
    write(root, "Packages/signal.luau", "return {}\n");
    write(root, "src/shared/util/math.luau", "return {}\n");
    write(
        root,
        "src/shared/util/geometry.luau",
        "local math = require(\"./math\")\nreturn math\n",
    );
    write(
        root,
        "src/server/main.server.luau",
        concat!(
            "-- entry point; require(\"./inside-comment\") must not be touched\n",
            "local Signal = require(\"@pkg/signal\")\n",
            "local math = require(\"../shared/util/math\") -- cross mount\n",
            "print(Signal, math)\n",
        ),
    );
    // A directory module with an init and a child
    write(
        root,
        "src/shared/pkg/init.luau",
        "local sub = require(\"@self/sub\")\nreturn sub\n",
    );
    write(root, "src/shared/pkg/sub.luau", "return 1\n");
    // Consumer of the directory module, sibling file
    write(
        root,
        "src/shared/consumer.luau",
        "return require(\"./pkg\")\n",
    );
    // Non code asset that must be copied through
    write(root, "src/shared/data.json", "{\"k\":1}\n");
    tmp
}

#[test]
fn processes_fixture_end_to_end() {
    let tmp = fixture();
    let root = tmp.path();
    let config = Config::load_or_default(root).unwrap();
    let outcome = pipeline::run(root, &config, true).unwrap();

    for d in &outcome.diags {
        eprintln!("{d}");
    }
    assert!(!outcome.has_errors(), "unexpected errors");

    // Alias expanded to a native @game require
    let main = read(root, "dist/server/main.server.luau");
    assert!(
        main.contains(r#"require("@game/ReplicatedStorage/Packages/signal")"#),
        "alias not expanded: {main}"
    );
    // Cross mount relative went absolute
    assert!(
        main.contains(r#"require("@game/ReplicatedStorage/shared/util/math")"#),
        "cross-mount require not rewritten: {main}"
    );
    // Comment untouched (splice preserves all other bytes)
    assert!(main.contains("require(\"./inside-comment\")"));
    // Trailing content preserved
    assert!(main.contains("print(Signal, math)"));

    // Same mount sibling stays relative (identical -> byte-identical output)
    let geometry = read(root, "dist/shared/util/geometry.luau");
    assert_eq!(geometry, "local math = require(\"./math\")\nreturn math\n");

    // @self pass-through
    let init = read(root, "dist/shared/pkg/init.luau");
    assert!(init.contains(r#"require("@self/sub")"#));

    // Sibling require of a directory module stays relative
    let consumer = read(root, "dist/shared/consumer.luau");
    assert_eq!(consumer, "return require(\"./pkg\")\n");

    // Non code file copied
    assert_eq!(read(root, "dist/shared/data.json"), "{\"k\":1}\n");

    // Derived build project generated with rerelativized paths
    let bp = outcome.build_project.expect("derived build project");
    let bp_json: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&bp).unwrap()).unwrap();
    assert_eq!(
        bp_json["tree"]["ReplicatedStorage"]["shared"]["$path"],
        "../dist/shared"
    );
    assert_eq!(
        bp_json["tree"]["ReplicatedStorage"]["Packages"]["$path"],
        "../Packages"
    );
    assert_eq!(
        bp_json["tree"]["ServerScriptService"]["$path"],
        "../dist/server"
    );
}

#[test]
fn idempotent_reprocessing() {
    let tmp = fixture();
    let root = tmp.path();
    let config = Config::load_or_default(root).unwrap();
    pipeline::run(root, &config, true).unwrap();
    let first = read(root, "dist/server/main.server.luau");

    // Process the dist tree as input, already-native requires pass through
    write(root, "src/server/main.server.luau", &first);
    let outcome = pipeline::run(root, &config, true).unwrap();
    for d in &outcome.diags {
        eprintln!("{d}");
    }
    assert!(!outcome.has_errors());
    assert_eq!(read(root, "dist/server/main.server.luau"), first);
}

#[test]
fn unknown_alias_is_error() {
    let tmp = fixture();
    let root = tmp.path();
    write(root, "src/shared/bad.luau", "return require(\"@nope/x\")\n");
    let config = Config::load_or_default(root).unwrap();
    let outcome = pipeline::run(root, &config, false).unwrap();
    assert!(outcome.has_errors());
    assert!(
        outcome
            .diags
            .iter()
            .any(|d| d.severity == Severity::Error && d.message.contains("unknown alias @nope"))
    );
}

#[test]
fn client_requiring_server_is_error() {
    let tmp = fixture();
    let root = tmp.path();
    // Client-marked script requiring a server only module
    write(root, "src/server/secret.luau", "return {}\n");
    write(
        root,
        "src/shared/ui.client.luau",
        "return require(\"@game/ServerScriptService/secret\")\n",
    );
    let config = Config::load_or_default(root).unwrap();
    let outcome = pipeline::run(root, &config, false).unwrap();
    assert!(outcome.has_errors());
    assert!(
        outcome
            .diags
            .iter()
            .any(|d| d.message.contains("does not replicate")
                || d.message.contains("cannot require from"))
    );
}

#[test]
fn absolute_into_starter_container_is_error() {
    let tmp = fixture();
    let root = tmp.path();
    write(
        root,
        "src/shared/bad_starter.luau",
        "return require(\"@game/StarterGui/hud/logic\")\n",
    );
    let config = Config::load_or_default(root).unwrap();
    let outcome = pipeline::run(root, &config, false).unwrap();
    assert!(outcome.has_errors());
    assert!(outcome.diags.iter().any(|d| d.message.contains("clones")));
}

#[test]
fn unprefixed_require_is_error() {
    let tmp = fixture();
    let root = tmp.path();
    write(
        root,
        "src/shared/legacy.luau",
        "return require(\"sibling\")\n",
    );
    let config = Config::load_or_default(root).unwrap();
    let outcome = pipeline::run(root, &config, false).unwrap();
    assert!(outcome.has_errors());
    assert!(
        outcome
            .diags
            .iter()
            .any(|d| d.message.contains("not RFC-valid"))
    );
}

#[test]
fn missing_target_warns_then_errors_under_strict() {
    let tmp = fixture();
    let root = tmp.path();
    write(
        root,
        "src/shared/dangling.luau",
        "return require(\"./ghost\")\n",
    );
    let config = Config::load_or_default(root).unwrap();
    let outcome = pipeline::run(root, &config, false).unwrap();
    assert!(
        !outcome.has_errors(),
        "missing target should be a warning by default"
    );
    assert!(
        outcome
            .diags
            .iter()
            .any(|d| d.severity == Severity::Warning)
    );

    write(
        root,
        "coldluau.toml",
        r#"
            [aliases]
            pkg = "@game/ReplicatedStorage/Packages"
            [requires]
            strict = true
        "#,
    );
    let config = Config::load_or_default(root).unwrap();
    let outcome = pipeline::run(root, &config, false).unwrap();
    assert!(
        outcome.has_errors(),
        "strict should upgrade missing-target to error"
    );
}

#[test]
fn luaurc_aliases_work_zero_config() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    write(
        root,
        "default.project.json",
        r#"{
            "name": "z",
            "tree": {
                "$className": "DataModel",
                "ReplicatedStorage": { "app": { "$path": "src" } }
            }
        }"#,
    );
    write(
        root,
        ".luaurc",
        r#"{ "aliases": { "util": "./src/util" } }"#,
    );
    write(root, "src/util/list.luau", "return {}\n");
    write(root, "src/main.luau", "return require(\"@util/list\")\n");

    // No coldluau.toml at all
    let config = Config::load_or_default(root).unwrap();
    let outcome = pipeline::run(root, &config, true).unwrap();
    for d in &outcome.diags {
        eprintln!("{d}");
    }
    assert!(!outcome.has_errors());
    let main = read(root, "dist/main.luau");
    // util maps into the same mount -> relative require
    assert_eq!(main, "return require(\"./util/list\")\n");
}

fn instance_fixture(indexing_style: &str) -> tempfile::TempDir {
    let tmp = fixture();
    let root = tmp.path();
    write(
        root,
        "coldluau.toml",
        &format!(
            "[aliases]\npkg = \"@game/ReplicatedStorage/Packages\"\n\n[requires]\ntarget = \"roblox-instance\"\nindexing_style = \"{indexing_style}\"\n"
        ),
    );
    tmp
}

#[test]
fn instance_target_find_first_child() {
    let tmp = instance_fixture("find_first_child");
    let root = tmp.path();
    let config = Config::load_or_default(root).unwrap();
    let outcome = pipeline::run(root, &config, true).unwrap();
    for d in &outcome.diags {
        eprintln!("{d}");
    }
    assert!(!outcome.has_errors());

    let main = read(root, "dist/server/main.server.luau");
    // Alias with a @game value becomes an absolute chain
    assert!(
        main.contains(
            r#"require(game:GetService("ReplicatedStorage"):FindFirstChild("Packages"):FindFirstChild("signal"))"#
        ),
        "alias not converted: {main}"
    );
    // Cross mount relative goes absolute too
    assert!(main.contains(
        r#"require(game:GetService("ReplicatedStorage"):FindFirstChild("shared"):FindFirstChild("util"):FindFirstChild("math"))"#
    ));

    // Same mount sibling becomes a script relative chain
    let geometry = read(root, "dist/shared/util/geometry.luau");
    assert!(
        geometry.contains(r#"require(script.Parent:FindFirstChild("math"))"#),
        "{geometry}"
    );

    // @self resolves to a child of the script
    let init = read(root, "dist/shared/pkg/init.luau");
    assert!(
        init.contains(r#"require(script:FindFirstChild("sub"))"#),
        "{init}"
    );
}

#[test]
fn instance_target_wait_for_child() {
    let tmp = instance_fixture("wait_for_child");
    let root = tmp.path();
    let config = Config::load_or_default(root).unwrap();
    let outcome = pipeline::run(root, &config, true).unwrap();
    assert!(!outcome.has_errors());
    let geometry = read(root, "dist/shared/util/geometry.luau");
    assert!(
        geometry.contains(r#"require(script.Parent:WaitForChild("math"))"#),
        "{geometry}"
    );
}

#[test]
fn instance_target_property_style() {
    let tmp = instance_fixture("property");
    let root = tmp.path();
    // A parenless require must get wrapped in parens
    write(
        root,
        "src/shared/parenless.luau",
        "return require \"./util/math\"\n",
    );
    let config = Config::load_or_default(root).unwrap();
    let outcome = pipeline::run(root, &config, true).unwrap();
    for d in &outcome.diags {
        eprintln!("{d}");
    }
    assert!(!outcome.has_errors());

    let main = read(root, "dist/server/main.server.luau");
    assert!(
        main.contains("require(game.ReplicatedStorage.Packages.signal)"),
        "{main}"
    );
    let geometry = read(root, "dist/shared/util/geometry.luau");
    assert!(
        geometry.contains("require(script.Parent.math)"),
        "{geometry}"
    );

    let parenless = read(root, "dist/shared/parenless.luau");
    assert!(
        parenless.contains("require (script.Parent.util.math)"),
        "{parenless}"
    );
}

#[test]
fn instance_style_accepts_kebab_alias() {
    let tmp = instance_fixture("property-instance");
    let config = Config::load_or_default(tmp.path()).unwrap();
    assert_eq!(
        config.requires.indexing_style,
        Some(coldluau::config::IndexingStyle::Property)
    );
}

#[test]
fn indexing_style_requires_instance_target() {
    let tmp = fixture();
    let root = tmp.path();
    write(
        root,
        "coldluau.toml",
        "[requires]\nindexing_style = \"property\"\n",
    );
    assert!(Config::load_or_default(root).is_err());
}

#[test]
fn path_target_for_lune() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    write(root, ".luaurc", r#"{ "aliases": { "lib": "./lib" } }"#);
    write(root, "lib/json.luau", "return {}\n");
    write(root, "coldluau.toml", "[requires]\ntarget = \"path\"\n");
    write(root, "src/main.luau", "return require(\"@lib/json\")\n");
    let config = Config::load_or_default(root).unwrap();
    let outcome = pipeline::run(root, &config, true).unwrap();
    for d in &outcome.diags {
        eprintln!("{d}");
    }
    assert!(!outcome.has_errors());
    assert_eq!(
        read(root, "dist/main.luau"),
        "return require(\"../lib/json\")\n"
    );
}

#[test]
fn quote_style_single_applies_everywhere() {
    let tmp = fixture();
    let root = tmp.path();
    write(
        root,
        "coldluau.toml",
        "[aliases]\npkg = \"@game/ReplicatedStorage/Packages\"\n\n[process]\nquotes = \"single\"\n",
    );
    let config = Config::load_or_default(root).unwrap();
    let outcome = pipeline::run(root, &config, true).unwrap();
    for d in &outcome.diags {
        eprintln!("{d}");
    }
    assert!(!outcome.has_errors());

    // rewritten require uses single quotes
    let main = read(root, "dist/server/main.server.luau");
    assert!(
        main.contains("require('@game/ReplicatedStorage/Packages/signal')"),
        "{main}"
    );
    // untouched relative require gets requoted too
    let geometry = read(root, "dist/shared/util/geometry.luau");
    assert!(geometry.contains("require('./math')"), "{geometry}");
    // @self passthrough requoted
    let init = read(root, "dist/shared/pkg/init.luau");
    assert!(init.contains("require('@self/sub')"), "{init}");
}

#[test]
fn quote_style_threads_into_instance_exprs() {
    let tmp = instance_fixture("find_first_child");
    let root = tmp.path();
    write(
        root,
        "coldluau.toml",
        concat!(
            "[aliases]\npkg = \"@game/ReplicatedStorage/Packages\"\n\n",
            "[process]\nquotes = \"single\"\n\n",
            "[requires]\ntarget = \"roblox-instance\"\n",
        ),
    );
    let config = Config::load_or_default(root).unwrap();
    let outcome = pipeline::run(root, &config, true).unwrap();
    assert!(!outcome.has_errors());
    let geometry = read(root, "dist/shared/util/geometry.luau");
    assert!(
        geometry.contains("require(script.Parent:FindFirstChild('math'))"),
        "{geometry}"
    );
}

#[test]
fn const_requires_rule() {
    let tmp = fixture();
    let root = tmp.path();
    write(
        root,
        "coldluau.toml",
        "[aliases]\npkg = \"@game/ReplicatedStorage/Packages\"\n\n[rules]\nconst_requires = true\n",
    );
    let config = Config::load_or_default(root).unwrap();
    let outcome = pipeline::run(root, &config, true).unwrap();
    for d in &outcome.diags {
        eprintln!("{d}");
    }
    assert!(!outcome.has_errors());

    let main = read(root, "dist/server/main.server.luau");
    assert!(
        main.contains("const Signal = require(\"@game/ReplicatedStorage/Packages/signal\")"),
        "{main}"
    );
    assert!(
        main.contains("const math = require(\"@game/ReplicatedStorage/shared/util/math\")"),
        "{main}"
    );
    // non local requires untouched
    let consumer = read(root, "dist/shared/consumer.luau");
    assert!(consumer.contains("return require(\"./pkg\")"), "{consumer}");
}

#[test]
fn unknown_rule_errors_with_milestone() {
    let tmp = fixture();
    let root = tmp.path();
    write(root, "coldluau.toml", "[rules]\nremove_types = true\n");
    let err = Config::load_or_default(root).unwrap_err().to_string();
    assert!(err.contains("remove_types") && err.contains("M2"), "{err}");
}

#[test]
fn remove_comments_rule() {
    let tmp = fixture();
    let root = tmp.path();
    write(
        root,
        "coldluau.toml",
        "[aliases]\npkg = \"@game/ReplicatedStorage/Packages\"\n\n[rules]\nremove_comments = true\n",
    );
    write(
        root,
        "src/shared/doc.luau",
        "--!strict\n-- a note\nlocal x = 1 -- trailing\nreturn x\n",
    );
    let config = Config::load_or_default(root).unwrap();
    let outcome = pipeline::run(root, &config, true).unwrap();
    assert!(!outcome.has_errors());

    let doc = read(root, "dist/shared/doc.luau");
    // Luau directives survive by default, plain comments do not
    assert!(doc.contains("--!strict"), "{doc}");
    assert!(!doc.contains("a note"), "{doc}");
    assert!(!doc.contains("trailing"), "{doc}");
    // line numbers preserved
    assert_eq!(doc.lines().count(), 4, "{doc}");
}

#[test]
fn append_text_comment_rule() {
    let tmp = fixture();
    let root = tmp.path();
    write(
        root,
        "coldluau.toml",
        concat!(
            "[aliases]\npkg = \"@game/ReplicatedStorage/Packages\"\n\n",
            "[rules.append_text_comment]\ntext = \"generated by coldluau\"\nlocation = \"start\"\n",
        ),
    );
    let config = Config::load_or_default(root).unwrap();
    let outcome = pipeline::run(root, &config, true).unwrap();
    assert!(!outcome.has_errors());
    let consumer = read(root, "dist/shared/consumer.luau");
    assert!(
        consumer.starts_with("-- generated by coldluau\n"),
        "{consumer}"
    );
    assert!(consumer.contains("require(\"./pkg\")"), "{consumer}");
}

#[test]
fn darklua_rule_names_get_useful_errors() {
    let tmp = fixture();
    let root = tmp.path();
    let cases = [
        ("rename_variables = true", "M2"),
        ("convert_require = true", "[requires]"),
        ("inject_global_value = true", "[defines]"),
        ("remove_spaces = true", "dense"),
    ];
    for (line, expect) in cases {
        write(root, "coldluau.toml", &format!("[rules]\n{line}\n"));
        let err = Config::load_or_default(root).unwrap_err().to_string();
        assert!(err.contains(expect), "{line} -> {err}");
    }
    // a name darklua does not have either
    write(root, "coldluau.toml", "[rules]\nmake_it_fast = true\n");
    let err = Config::load_or_default(root).unwrap_err().to_string();
    assert!(err.contains("unknown rule"), "{err}");
}

#[test]
fn cache_skips_unchanged_files_and_notices_edits() {
    let tmp = fixture();
    let root = tmp.path();
    let config = Config::load_or_default(root).unwrap();

    let first = pipeline::run(root, &config, true).unwrap();
    assert_eq!(first.stats.files_cached, 0, "cold build caches nothing");

    let second = pipeline::run(root, &config, true).unwrap();
    assert_eq!(
        second.stats.files_cached, second.stats.files_processed,
        "warm build should skip everything"
    );

    // editing one file rebuilds exactly that file
    write(
        root,
        "src/shared/util/geometry.luau",
        "local math = require(\"./math\")\nreturn { math }\n",
    );
    let third = pipeline::run(root, &config, true).unwrap();
    assert_eq!(
        third.stats.files_cached,
        third.stats.files_processed - 1,
        "only the edited file should rebuild"
    );
    assert!(read(root, "dist/shared/util/geometry.luau").contains("{ math }"));
}

#[test]
fn luaurc_change_invalidates_the_whole_cache() {
    let tmp = fixture();
    let root = tmp.path();
    let config = Config::load_or_default(root).unwrap();
    pipeline::run(root, &config, true).unwrap();
    let warm = pipeline::run(root, &config, true).unwrap();
    assert!(warm.stats.files_cached > 0);

    // a .luaurc changes how requires resolve for files that did not change
    write(root, ".luaurc", r#"{ "aliases": { "extra": "./src" } }"#);
    let after = pipeline::run(root, &config, true).unwrap();
    assert_eq!(
        after.stats.files_cached, 0,
        "resolution inputs changed, everything must rebuild"
    );
}

#[test]
fn deleting_a_source_removes_its_output() {
    let tmp = fixture();
    let root = tmp.path();
    let config = Config::load_or_default(root).unwrap();
    write(root, "src/shared/temporary.luau", "return 1\n");
    pipeline::run(root, &config, true).unwrap();
    assert!(root.join("dist/shared/temporary.luau").exists());

    fs::remove_file(root.join("src/shared/temporary.luau")).unwrap();
    let after = pipeline::run(root, &config, true).unwrap();
    assert!(
        !root.join("dist/shared/temporary.luau").exists(),
        "stale output should be pruned"
    );
    assert_eq!(after.stats.files_pruned, 1);
}

#[test]
fn check_reports_syntax_errors() {
    let tmp = fixture();
    let root = tmp.path();
    write(root, "src/shared/broken.luau", "local x = = 1\n");
    let config = Config::load_or_default(root).unwrap();
    let outcome = pipeline::run(root, &config, false).unwrap();
    assert!(outcome.has_errors());
    assert!(
        outcome
            .diags
            .iter()
            .any(|d| d.message.contains("syntax error")),
        "{:?}",
        outcome.diags.iter().map(|d| &d.message).collect::<Vec<_>>()
    );
}

#[test]
fn rules_that_touch_the_same_byte_do_not_corrupt_output() {
    // add_luau_directive inserts at byte 0 and const_requires replaces the
    // `local` that also starts at byte 0
    let tmp = fixture();
    let root = tmp.path();
    write(
        root,
        "coldluau.toml",
        concat!(
            "[aliases]\npkg = \"@game/ReplicatedStorage/Packages\"\n\n",
            "[rules]\nconst_requires = true\nadd_luau_directive = \"strict\"\n",
        ),
    );
    write(
        root,
        "src/shared/first.luau",
        "local S = require(\"@pkg/signal\")\nreturn S\n",
    );
    let config = Config::load_or_default(root).unwrap();
    let outcome = pipeline::run(root, &config, true).unwrap();
    assert!(!outcome.has_errors());
    assert_eq!(
        read(root, "dist/shared/first.luau"),
        "--!strict\nconst S = require(\"@game/ReplicatedStorage/Packages/signal\")\nreturn S\n"
    );
}
