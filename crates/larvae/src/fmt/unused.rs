/*!
Finds the required modules that nothing uses, for `unused_imports`.

The decision lives here and not in the emitter for the same reason
`require_binding` does: it needs the scope resolution that the linter builds,
and the emitter should receive an answer rather than the machinery.

What counts as a use is the whole question, and one case decides whether the
option is safe to turn on. A name that only a type reads is used:

```luau
const jecs = require("@pkg/jecs")

type Component = jecs.Component
```

The parser consumes type syntax for its extent and does not interpret it, so
a naive walk of the expressions never sees `jecs` again and calls the import
dead. To delete that line breaks the type below it. The resolver already
recovers a reference from inside a type, because `unused_variable` had the
same defect and it was fixed there. This module asks the resolver and adds no
rule of its own, so the lint and the formatter cannot disagree.
*/

use std::collections::{HashMap, HashSet};

use crate::syntax::ast::*;
use crate::syntax::lexer::Tok;

use super::config::UnusedImports;

#[derive(Debug, Default)]
pub struct Plan {
    /// The name token of a binding, and the text that replaces it.
    pub renames: HashMap<u32, String>,
    /// The first token of each declaration to drop.
    pub removals: HashSet<u32>,
}

impl Plan {
    pub fn is_empty(&self) -> bool {
        self.renames.is_empty() && self.removals.is_empty()
    }
}

pub fn plan(
    src: &str,
    toks: &[Tok],
    chunk: &Chunk,
    mode: UnusedImports,
    already: &super::rebind::Rebindings,
) -> Plan {
    let mut out = Plan::default();

    if mode == UnusedImports::Ignore {
        return out;
    }

    let names = crate::lint::scope::resolve(src, toks, chunk);

    let mut locals = Vec::new();
    super::rebind::collect(chunk, &mut locals);

    for local in &locals {
        let Some(binding) = super::rebind::single_require_binding(src, toks, local) else {
            continue;
        };

        let name = toks[binding.name.start as usize].text(src);

        /*
        A name that opens with `_` already says it is unused, which is the
        convention `unused_variable` reads. Under `underscore` there is
        nothing left to do, and under `remove` the author marked the line on
        purpose and asked for it to stay.
        */
        if name.starts_with('_') {
            continue;
        }

        let used = names
            .by_token
            .get(&binding.name.start)
            .and_then(|&i| names.bindings.get(i))
            .is_none_or(|b| !b.reads.is_empty());

        if used {
            continue;
        }

        match mode {
            UnusedImports::Underscore => {
                out.renames.insert(binding.name.start, format!("_{name}"));
            }

            /*
            A declaration that `require_binding` or `prefer_const` already
            rewrote is left alone. Both of those options decided this line
            says something, and a removal that fights them would make the
            output depend on which pass ran first.
            */
            UnusedImports::Remove if !already.contains_key(&local.keyword.start) => {
                out.removals.insert(local.span.start);
            }

            _ => {}
        }
    }

    out
}
