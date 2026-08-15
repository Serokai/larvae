# Changelog

Notable changes land here. Format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), versions follow
[semver](https://semver.org/spec/v2.0.0.html).

## Unreleased

### Added

- Minification: `generator = "dense"` re-emits the output tokens with the
  least whitespace that lexes the same, so the program cannot change
  meaning. The `[minify]` table tunes it, `column_span` keeps line numbers
  useful and `rename_variables` shortens locals for dense builds only
- `generator = "readable"` prints the output through the formatter, in the
  `[fmt]` style of the project. The generator also prints the
  `larvae bundle` output
- A root `exclude` / `include` pair in `larvae.toml` that every command
  inherits. The include of an area wins over every exclude, the exclude of
  an area wins over the root include, and the root include cancels the root
  exclude alone
- Root short forms `input`, `output`, and `target` for the keys every
  project sets, so the first line of a config needs no table header.
  Writing both spellings of one key is an error
- `extends`, a base config that a file layers over by relative path, with
  the merge rules of `[profile]`. Chains resolve, loops are refused, and a
  base can hold the profiles of a whole workspace

### Changed

- `[process] include` and `[process] exclude` match relative to the project
  root now, like every other list, and the exclude follows the same
  directory-name rule. Patterns written against `input` need respelling
- `larvae init` writes the root short forms and stops listing every default
  as a comment; the docs and the schema hold the full lists
- Smaller dependency tables behind the same behavior: URL parsing keeps the
  compact unicode backend, and the TOML parser dropped its edit layer

## 0.1.1 - 2026-08-14

### Added

- The require graph and `larvae check` gates under `[check]`: `cycles`,
  `unused_modules`, `early_require`, and `entries`
- `larvae bundle`: one tree-shaken file with a lazy module registry, so
  bundling cannot move a side effect and a load-time cycle errors naming
  the module
- `larvae sync-luaurc` writes the merged aliases back into `.luaurc`
- `[fmt] require_binding` selects the keyword that binds a required module,
  and the `non_const_require` lint reports the requires that `const` cannot
  bind

### Fixed

- `const_requires` skips a binding that a later statement reassigns, which
  would have been a syntax error under Luau's `const`
- Linux release builds ship, built against musl with a C++ toolchain

### Performance

- The require graph is harvested only when `check` or `bundle` reads it, so
  a plain build pays nothing for it
- The lint report renders into one buffer and one write

## 0.1.0 - 2026-08-13

### Added

- Require rewriting with three output targets, native Roblox string requires,
  filesystem paths for Lune, and Instance expressions with `find_first_child`,
  `wait_for_child` or `property` indexing
- Aliases from `larvae.toml` and `.luaurc`, merged per key, with chain and
  cycle handling
- Realm and container validation, client code cannot require server only
  containers and Starter containers only ever get relative requires
- Rojo integration, mounts derived from `default.project.json` and a build
  project written to `.larvae/build.project.json`
- A Luau parser and printer, round trips byte for byte, used by `check` to
  report syntax errors
- Incremental builds keyed on a resolution epoch, plus `process --watch`
- Formatting with stylua parity and options beyond it, and linting with the
  selene rule set, both reading the config files those tools leave
- The worm system: extensions in three forms, `luau`, `wasm`, and `native`.
  A worm transforms, formats, and lints the files it claims: a format reply
  is a layout document that larvae renders in the style of the project, and
  a lint reply is stamped with levels from `[lint.rules]`
- A worm namespaces its options and lints under its own key,
  `[fmt.<worm>]` and `[lint.rules.<worm>]`, and each lint reads
  `worm.name` in messages, in `--explain`, and in allow comments
- Inheritance controls per worm: builtin lints on claimed files,
  `[worms.<name>.inherit]` with `lints_only`, `lints_except`, and
  `fmt_except`
- A cargo install channel for worms, `cargo = "crate@version"`, beside the
  GitHub release and path channels
- A generated per-project schema, connected to the editor by
  `larvae self code`
- Rules, `const_requires`, `remove_comments`, `append_text_comment` and
  `add_luau_directive`, with every darklua rule name accepted
- `larvae init`, `larvae self code`, and `larvae self install`, `update`
  and `uninstall`

### Fixed

- `larvae-worm` links into a worm that is not wasm. The node API of the wasm
  form declared its host functions on every target, so `link.exe` refused a
  native worm over nine unresolved names and bound the tenth, `remove`, to the
  function of the C library that deletes a file
