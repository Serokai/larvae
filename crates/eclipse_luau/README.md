# eclipse_luau

A small, fast Luau syntax layer: lexer, parser, AST, require-site scanner,
lossless printer, and a dense re-emitter. It is the parser under
[larvae](https://github.com/larvae-luau/larvae), published on its own so any
tool can use it.

## The contract

- **Byte ranges are identity.** Every token and node carries byte offsets
  into the source, never a line and a column. A consumer derives those on
  demand. So a transform splices byte ranges against the original text with
  no loss, which is what larvae's whole edit model is built on.
- **The round trip holds.** `print(parse(src)) == src`, byte for byte, for
  every file the parser accepts. A fuzz target holds this invariant, and the
  vendored Luau conformance corpus must round-trip on every commit.
- **Parallelism is file level.** One file parses fast and single threaded.
  `parse_many` spreads a list over a thread pool.

## Syntax coverage

Shipped Luau plus the merged RFCs: classes (`class`/`open`/`extends`,
`public` fields, the metamethod whitelist), export by value (`export
local/const/function`), and integer literals (`123i`, `0xABi`, `0b1010i`).
The contextual keywords stay contextual, so code that uses those words as
names parses as it always did.

## Benchmarks

```
cargo bench -p eclipse_luau                             # this crate alone
cargo bench -p eclipse_luau --features full-moon-bench  # against full_moon
BENCH_CORPUS=/path/to/big/codebase cargo bench ...      # a real corpus
```

No performance claims stand in this README until the corpus numbers are
recorded from a fixed machine and published here.
