# Contributing

Thanks for looking. Issues and pull requests are both welcome.

## Getting set up

```bash
cargo build
cargo test
```

That is the whole setup. No code generation step, no submodules, no network
during tests.

Before you push, run what CI runs.

```bash
cargo fmt
cargo clippy --all-targets -- -D warnings
cargo test
```

Clippy runs with warnings denied, so a warning fails the build. If you touch
the lexer or the parser, also run the conformance suite on its own, it is the
slow one and the one most likely to catch you out.

```bash
cargo test --test parser
```

## Commit messages

We use [Conventional Commits](https://www.conventionalcommits.org). The
subject line is `type(optional scope): summary`, written in the imperative,
lowercase, and no trailing period.

```
feat(requires): resolve aliases through the luaurc chain
fix(lexer): stop long strings from swallowing the closing bracket
docs: write the contributing guide
```

Types we use:

| type | when |
|---|---|
| `feat` | new behavior a user can see |
| `fix` | a bug fix |
| `perf` | faster with no behavior change |
| `refactor` | internal shuffling, no behavior change |
| `docs` | documentation only |
| `test` | tests only |
| `build` | dependencies, cargo config, packaging |
| `ci` | workflow changes |
| `chore` | everything else |

Scopes are optional and free form. The ones that show up most are `requires`,
`lexer`, `parser`, `rules`, `rojo`, `cache`, `cli`, `config` and `ui`.

Put a body on anything that is not obvious from the subject. Explain why, not
what, the diff already says what. Wrap it at 72 columns.

Breaking changes get a `!` after the type, plus a `BREAKING CHANGE:` line in
the body saying what to do instead.

```
feat(config)!: move requires out of the rules list

BREAKING CHANGE: rewriting is configured under [requires] now, the
convert_require rule no longer exists.
```

## Writing style

This applies to comments, docs, and anything a user reads.

No em dashes. Use a comma, or split the sentence. Keep sentences short and
plain. Say "ex:" instead of "e.g.". Skip the trailing period on short labels
and table cells. Comments should read like a person wrote them, so explain the
reasoning rather than restating the code.

Groups of two or more line comments become a block comment. One liners stay
line comments.

## Where things live

| path | what is in it |
|---|---|
| `src/syntax/` | lexer, AST, parser, printer |
| `src/requires/` | require resolution and the DataModel map |
| `src/project/` | Rojo project files and `.luaurc` |
| `src/rules.rs` | builtin transforms |
| `src/commands/` | one file per CLI command |
| `src/pipeline.rs` | discovery, the parallel loop, writing output |
| `src/ui.rs` | all theming, the brand color lives here and nowhere else |
| `tests/` | end to end and parser conformance |
| `fuzz/` | cargo fuzz targets, nightly only |

`plan.md` is the design document and the roadmap. Read the section that covers
your change before you start, most of the surprising decisions are explained
there, especially the require semantics and the DataModel rules.

## Adding a rule

Rules live in `src/rules.rs`. A new one needs four things, the implementation,
an entry in `RulesConfig`, an entry in `coldluau.schema.json` so editors know
about it, and a line in `coldluau.example.toml`. Add a test in `tests/e2e.rs`
that shows the before and after.

Rule names match darklua wherever a darklua equivalent exists. That is
deliberate, a config should port over without renaming anything.

## Touching the parser

Two invariants hold and the test suite enforces both. Parsing then printing
returns the input byte for byte, and every block's statements tile its token
span with no holes. If you add a node, add it to the coverage walk in
`printer.rs` and add a snippet to the corpus in `tests/parser.rs`.

Recursion is depth guarded on purpose. Deeply nested input must produce a
clean error, never a stack overflow.

## Reporting a bug

The most useful report is a small Luau file plus the config that mishandles
it, and what you expected instead. If coldluau emitted a require that fails at
runtime, say which container the requiring script lives in, that detail is
usually the whole answer.
