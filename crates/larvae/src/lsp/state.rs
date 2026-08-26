/*!
What the server holds between requests: the config of the project and its
worms, and the reloads that keep both current.
*/

use std::io::Write;
use std::path::Path;

use anyhow::Result;
use serde_json::{Value, json};

use crate::commands::fmt::pool_with;
use crate::fmt::FmtConfig;
use crate::lint::LintConfig;
use crate::worm::pool::Pool;

use super::uri::path_of_uri;
use super::{Server, rpc};

impl Server {
    pub(super) fn initialize(&mut self, params: &Value, out: &mut impl Write) -> Result<()> {
        self.root = params["rootUri"]
            .as_str()
            .and_then(path_of_uri)
            .or_else(|| {
                params["workspaceFolders"][0]["uri"]
                    .as_str()
                    .and_then(path_of_uri)
            });

        self.load_config(out)
    }

    /*
    Read the config of the project, with the defaults as the fallback.

    A broken config does not stop the server, because the user edits that
    config in the editor, and a server that refuses to start over an
    incomplete file is not usable. But silence is wrong too: a project that
    formats with defaults for a whole session looks broken in a quieter
    way. So a config that fails to resolve raises one editor notification,
    and the message carries the reason. A project without a larvae.toml is
    the zero config case and raises nothing.
    */
    pub(super) fn load_config(&mut self, out: &mut impl Write) -> Result<()> {
        // The load of the worms takes `&mut self`, so the root arrives as a copy.
        let Some(root) = self.root.clone() else {
            return Ok(());
        };

        let path = root.join("larvae.toml");

        let project = match path.exists() {
            true => match crate::config::Config::load(&path) {
                Ok(cfg) => Some(cfg),

                Err(e) => {
                    warn(
                        out,
                        &format!("{e:#}; larvae serves defaults until it loads"),
                    )?;

                    None
                }
            },

            false => None,
        };

        if let Some(project) = &project {
            self.lsp = project.lsp.clone();
            self.aliases = project.alias_map();
        }

        /*
        The analyzer receives the DataModel map, so `@game` resolves.

        The map is built the way the pipeline builds it, from the same two
        sources: `[requires.mounts]` and the rojo project. So the editor and
        `larvae process` answer one require the same way, which is the point
        of reading it from the config rather than guessing.

        A project with no config has no map, and a diagnostic about a broken
        mount belongs to `larvae check` and not to a keystroke, so the
        diagnostics of the build go nowhere here.
        */
        if let Some(analysis) = self.analysis.borrow_mut().as_mut() {
            /*
            A project with no larvae.toml still gets its mounts. The rojo
            project file alone describes a DataModel, and a zero config
            project is the common Roblox case.
            */
            let fallback = crate::config::Config::default();
            let cfg = project.as_ref().unwrap_or(&fallback);

            let rojo = crate::project::rojo::find_project(&root, cfg.rojo.project.as_deref())
                .and_then(|path| crate::project::rojo::load(&path).ok());

            // A broken mount is a `larvae check` diagnostic, not a keystroke one.
            let mut ignored = Vec::new();

            analysis.set_mounts(crate::pipeline::setup::mount_table(
                &root,
                cfg,
                rojo.as_ref(),
                &mut ignored,
            ));
        }

        match FmtConfig::discover(&root, project.as_ref().and_then(|c| c.fmt.as_ref())) {
            Ok(cfg) => self.fmt = cfg,

            Err(e) => warn(out, &format!("{e:#}"))?,
        }

        match LintConfig::discover(&root, project.as_ref().and_then(|c| c.lint.as_ref())) {
            Ok(cfg) => {
                // The root lists apply here too, so the editor and the command agree.
                let (root_in, root_ex) = project
                    .as_ref()
                    .map(|c| (c.include.as_slice(), c.exclude.as_slice()))
                    .unwrap_or((&[], &[]));

                self.excluded = cfg
                    .excludes_under(&root, root_in, root_ex)
                    .unwrap_or_default();
                self.lint = cfg;
            }

            Err(e) => warn(out, &format!("{e:#}"))?,
        }

        self.load_worms(&root);

        Ok(())
    }

    /*
    Read the worms of the project.

    The server keeps no worm when the build fails, and then serves the Luau
    files as before. A user who edits `[worms]` breaks that table for some
    keystrokes, and an editor that stops at each of them is not usable.

    The build also checks the `[fmt]` table against the options that the
    worms declare, and fills each missing option. So the server takes the new
    fmt config only when the build succeeds.
    */
    fn load_worms(&mut self, root: &Path) {
        let mut fmt = self.fmt.clone();

        // the editor never downloads a worm, because a keystroke cannot wait
        match pool_with(root, None, &mut fmt, crate::worm::registry::Fetch::Quiet) {
            Ok(pool) => {
                self.fmt = fmt;
                self.worm_stamp = stamp_of(&pool);
                self.worms = pool;
            }

            Err(_) => self.worms = no_worms(),
        }

        self.install_lsp_hooks();
    }

    /*
    The worm hooks, installed into the analyzer behind the seam.

    Tier 1 goes in as module hooks: the analyzer asks the pool before its
    default resolution, so a worm's lowered module is what the type system
    reads. Tier 2 goes in as declaration text. The install runs after every
    worm load, so a rebuilt worm re-declares, which is the invalidation the
    plan demands: a worm change behaves like a file change.

    Without an analyzer this is a no-op, and the pool's own lint and format
    hooks serve as before.
    */
    pub(super) fn install_lsp_hooks(&mut self) {
        let mut analysis = self.analysis.borrow_mut();

        let Some(analysis) = analysis.as_mut() else {
            return;
        };

        if !self.worms.has_lsp_hooks() {
            return;
        }

        let resolve_pool = self.worms.clone();
        let load_pool = self.worms.clone();

        analysis.set_module_hooks(crate::lsp::analysis::ModuleHooks {
            resolve: Box::new(move |from, spec| {
                resolve_pool.lsp_resolve(&from.to_string_lossy(), spec)
            }),
            load: Box::new(move |path| {
                load_pool
                    .lsp_load_any(path)
                    .map(|r| crate::lsp::analysis::plain_view(&r.source).into_owned())
            }),
        });

        for decl in self.worms.lsp_declarations() {
            analysis.definitions(&decl.name, &decl.source);
        }
    }

    /*
    Say once when a worm widens claim-only serving, because the user wrote
    that setting and the server is overriding it with a reason.
    */
    pub(super) fn notice_widened_serving(&self, out: &mut impl Write) -> Result<()> {
        if !self.lsp.claim_only || !self.worms.lsp_serves_luau() {
            return Ok(());
        }

        let names: Vec<&str> = self
            .worms
            .specs()
            .iter()
            .filter(|s| s.manifest.lsp.serves_luau)
            .map(|s| s.manifest.name.as_str())
            .collect();

        rpc::notify(
            out,
            "window/showMessage",
            json!({
                "type": 3,
                "message": format!(
                    "larvae serves every Luau file here, although [lsp] claim_only is on: worm `{}` answers inside plain Luau",
                    names.join("`, `")
                ),
            }),
        )
    }

    /*
    Rebuild the pool when a worm changed on disk.

    A worm author rebuilds a path worm and expects the next keystroke to use
    it. The command line reads the directory on every run, and a server that
    holds the first build all session would answer with a stale worm. The
    check costs one stat per worm artifact, so the server runs it before each
    request that a worm can answer.
    */
    pub(super) fn refresh_worms(&mut self) {
        let Some(root) = self.root.clone() else {
            return;
        };

        if self.worms.is_empty() {
            return;
        }

        if stamp_of(&self.worms) != self.worm_stamp {
            self.load_worms(&root);
        }
    }
}

/*
One warning toast in the editor; `window/showMessage` type 2 is Warning.

The protocol allows this notification before the reply to `initialize`, so
a config that is broken at startup reports right away.
*/
fn warn(out: &mut impl Write, message: &str) -> Result<()> {
    rpc::notify(
        out,
        "window/showMessage",
        json!({ "type": 2, "message": message }),
    )
}

/*
The modification time and the size of the entry of each worm.

A rebuilt artifact changes both on every real toolchain, and two stat calls
per worm cost nothing next to a lint pass.
*/
fn stamp_of(pool: &Pool) -> Vec<(std::path::PathBuf, Option<std::time::SystemTime>, u64)> {
    pool.specs()
        .iter()
        .map(|spec| {
            let entry = spec.dir.join(&spec.manifest.entry);
            let meta = std::fs::metadata(&entry).ok();

            (
                entry,
                meta.as_ref().and_then(|m| m.modified().ok()),
                meta.map(|m| m.len()).unwrap_or(0),
            )
        })
        .collect()
}

/// A pool with no worm in it. Every file then takes the Luau route.
pub(super) fn no_worms() -> Pool {
    Pool::new(Vec::new(), 1)
}
