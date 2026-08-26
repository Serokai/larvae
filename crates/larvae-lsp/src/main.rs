/*!
The larvae language server binary.

The server itself lives in the larvae crate, and `larvae lsp` runs it with
lint and format alone. This binary plugs Luau's analysis frontend into the
server's seam, so hover, completions, and type diagnostics join in. Built
without the `analyzer` feature, the binary is the same server as the
subcommand, which keeps a plain workspace build away from the C++.
*/

#[cfg(feature = "analyzer")]
mod analyzer;

// Pure path logic, so it compiles and tests without the vendored C++.
mod resolve;

fn main() -> std::process::ExitCode {
    #[cfg(feature = "analyzer")]
    let analysis =
        Some(Box::new(analyzer::LuauAnalysis::new()) as Box<dyn larvae::lsp::analysis::Analysis>);

    #[cfg(not(feature = "analyzer"))]
    let analysis = None;

    match larvae::lsp::run_with(analysis) {
        Ok(()) => std::process::ExitCode::SUCCESS,

        Err(e) => {
            eprintln!("larvae-lsp: {e:#}");

            std::process::ExitCode::FAILURE
        }
    }
}
