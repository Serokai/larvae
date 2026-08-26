/*!
The seam where a type analyzer plugs into the server.

The server in this crate lints and formats. The larvae-lsp binary adds
Luau's real analysis frontend, and this trait is the whole boundary
between them: the server calls these five methods and knows nothing about
the shim, the C++, or the vendored build behind them. `larvae lsp` runs
with no analyzer and serves exactly what it always served.

Positions cross this boundary as byte offsets, in both directions. The
line and column conversion of the protocol happens in the server, once,
at the edge.
*/

use std::path::Path;

/// One diagnostic from the analyzer, byte addressed
pub struct AnalysisDiag {
    pub span: (u32, u32),
    /// 1 is Error and 2 is Warning, the numbering of the protocol
    pub severity: u8,
    pub message: String,
    pub code: Option<String>,
}

/// One completion the analyzer offers at a position
pub struct AnalysisCompletion {
    pub label: String,
    /// The protocol's CompletionItemKind, ex: 5 is Field, 3 is Function
    pub kind: u8,
    pub detail: Option<String>,
}

/*
The module hooks the server installs before the first request.

Resolve answers a require spec from a module, or passes. Load answers the
text the analyzer should see for a path the hooks resolved, with the span
map back onto the original. Both run on the analyzer's hot path, so the
implementations behind them are resident worms, not spawns.
*/
pub type ResolveHook = Box<dyn Fn(&Path, &str) -> Option<String> + Send>;
pub type LoadHook = Box<dyn Fn(&str) -> Option<String> + Send>;

pub struct ModuleHooks {
    pub resolve: ResolveHook,
    pub load: LoadHook,
}

pub trait Analysis: Send {
    /// Install the module hooks; the server calls this once per worm load
    fn set_module_hooks(&mut self, hooks: ModuleHooks) {
        let _ = hooks;
    }

    /*
    The service names the platform knows, for auto-import completions.

    The analyzer reads them from its definitions, so the list and the
    types cannot drift. A server without an analyzer offers no service
    imports, which is the honest answer.
    */
    fn services(&mut self) -> Vec<String> {
        Vec::new()
    }

    /// Load one .d.luau declaration into the global scope
    fn definitions(&mut self, name: &str, source: &str) -> bool {
        let _ = (name, source);

        false
    }

    /// Give the analyzer the text of one open document
    fn open(&mut self, path: &Path, text: &str);

    /// Type diagnostics for one document
    fn check(&mut self, path: &Path) -> Vec<AnalysisDiag>;

    /// The type at a byte offset, rendered for a hover card
    fn hover(&mut self, path: &Path, at: u32) -> Option<String>;

    /// Completions at a byte offset
    fn completions(&mut self, path: &Path, at: u32) -> Vec<AnalysisCompletion>;

    /// Drop the cached state of one document and its dependents
    fn invalidate(&mut self, path: &Path);
}
