<div align="center">

<img src="assets/coldluautransparent.png" alt="coldluau" width="140">

# coldluau

**A fast Luau preprocessor. Aliases in, Roblox native string requires out.**

[![CI](https://github.com/coldluau/cli/actions/workflows/ci.yml/badge.svg)](https://github.com/coldluau/cli/actions/workflows/ci.yml)
[![Release](https://img.shields.io/github/v/release/coldluau/cli?color=87FDFF&label=release)](https://github.com/coldluau/cli/releases/latest)
[![Downloads](https://img.shields.io/github/downloads/coldluau/cli/total?color=87FDFF&label=downloads)](https://github.com/coldluau/cli/releases)
[![License](https://img.shields.io/badge/license-MIT-87FDFF)](LICENSE.md)

</div>

Roblox shipped native string requires, and `@game/...` went live in early 2026.
Nothing generated them. coldluau does.

```lua
-- what you write
local Signal = require("@pkg/signal")

-- what ships
local Signal = require("@game/ReplicatedStorage/Packages/signal")
```

```toml
# coldluau.toml
[aliases]
pkg = "@game/ReplicatedStorage/Packages"
```

That is the whole idea. No Instance chains, no sourcemap, and the output stays
readable in a diff.

## Why not darklua

darklua is good software, and coldluau accepts every one of its rule names so
porting a config is mostly copy and paste. Five things are different.

**It emits native string requires.** darklua only ever produced Instance
chains like `script.Parent:FindFirstChild("Foo")`. coldluau still can, with
all three indexing styles, but it does not have to.

**It refuses to emit requires that break at runtime.** Code under
StarterPlayerScripts runs as a clone, so an absolute `@game/StarterPlayer/...`
require resolves to the template instead of the copy and module state quietly
duplicates. Client code cannot reach ServerScriptService at all. coldluau maps
both ends of every require into the DataModel and errors on both cases.
Nothing else in the ecosystem checks this, darklua and luau lsp included.

**It is faster by architecture, not by tuning.** Files are processed in
parallel, rewrites are byte range splices rather than a full reprint, and
there is an incremental cache. darklua is single threaded and has been since
the issue was filed in 2021.

| files | coldluau cold | coldluau warm | darklua | speedup |
|---:|---:|---:|---:|---:|
| 3000 | 27 ms | 14 ms | 424 ms | 15.7x |
| 5000 | 44 ms | 24 ms | 722 ms | 16.4x |
| one 3.5 MB file | 21 ms | 3 ms | 1375 ms | 65.4x |

darklua ran with an empty rule list, so it only parsed and reprinted while
coldluau did the full job. Reproduce with `scripts/bench.sh`.

**One Rojo project file.** The usual setup keeps two nearly identical project
files, one pointed at source for sourcemaps and one pointed at the build
output for serving. coldluau derives the second from the first and keeps it
fresh, so you edit one file.

**It never runs rojo.** Serving is rojo's job. coldluau writes
`.coldluau/build.project.json` and stops there.

## Install

```bash
cargo install --path .
coldluau self install
```

`self install` copies the binary to `~/.coldluau/bin` and prints the line to
add to your shell profile. `coldluau self update` pulls the latest release
later on.

## Getting started

```bash
cd my-rojo-project
coldluau init      # writes coldluau.toml, offers to update .gitignore
coldluau process   # writes dist/ and .coldluau/build.project.json
rojo serve .coldluau/build.project.json
```

A project that already has a `default.project.json` and `.luaurc` aliases
needs no config at all. Mounts come from the project file, aliases come from
`.luaurc`, and `coldluau process` just works.

While editing, `coldluau process --watch` rebuilds on save, mirrors deletions,
and keeps the last good output when a file will not lex, so a half typed save
does not cascade require failures into a live Studio session.

## Commands

| command | what it does |
|---|---|
| `coldluau process` | rewrite requires into the output directory |
| `coldluau process --watch` | the same, on every save |
| `coldluau check` | validate requires and syntax, write nothing, exit non zero on errors |
| `coldluau init` | scaffold a config |
| `coldluau schema` | add the schema line for editor completion |
| `coldluau self install` | manage the install, with `update` and `uninstall` |

`check` is the CI gate. It reports unresolvable requires, realm violations,
alias cycles and syntax errors, and counts the dynamic requires it left alone
on purpose.

## Configuration

Every key is optional. Run `coldluau schema` for completion and hover docs in
any editor with Even Better TOML or Taplo.

```toml
[aliases]
pkg = "@game/ReplicatedStorage/Packages"

[process]
input = "src"
output = "dist"
quotes = "preserve"        # or double, or single

[requires]
target = "roblox-string"   # or path for Lune, or roblox-instance
strict = false

[rules]
const_requires = true      # local X = require(...) becomes const X = require(...)
add_luau_directive = "strict"
```

Unknown keys are hard errors, and keys for features that do not exist yet tell
you which release they land in. Nothing is silently ignored.

## Status

Requires, the Rojo integration, the parser, the cache and watch mode all work
today. Next up, reading Instance requires as input so existing codebases can
convert, compile time constants, build profiles, and the rest of the rules now
that the parser exists. After that, bundling with documented module init
order, cross module dead code elimination, and transforms you write yourself
in Luau.

## License

MIT, see [LICENSE.md](LICENSE.md).
