/*!
What the server holds between requests: the config of the project and its
worms, and the reloads that keep both current.
*/

use std::path::Path;

use serde_json::Value;

use crate::commands::fmt::pool_with;
use crate::fmt::FmtConfig;
use crate::lint::LintConfig;
use crate::worm::pool::Pool;

use super::Server;
use super::uri::path_of_uri;

impl Server {
    pub(super) fn initialize(&mut self, params: &Value) {
        self.root = params["rootUri"]
            .as_str()
            .and_then(path_of_uri)
            .or_else(|| {
                params["workspaceFolders"][0]["uri"]
                    .as_str()
                    .and_then(path_of_uri)
            });

        self.load_config();
    }

    /*
    Read the config of the project, with the defaults as the fallback.

    The server does not report a broken config here. The user edits that
    config in the editor. A server that refuses to start because the file is
    incomplete is worse than one that formats with defaults until the save.
    */
    pub(super) fn load_config(&mut self) {
        // The load of the worms takes `&mut self`, so the root arrives as a copy.
        let Some(root) = self.root.clone() else {
            return;
        };

        let project = crate::config::Config::load(&root.join("larvae.toml")).ok();

        if let Ok(cfg) = FmtConfig::discover(&root, project.as_ref().and_then(|c| c.fmt.as_ref())) {
            self.fmt = cfg;
        }

        if let Ok(cfg) = LintConfig::discover(&root, project.as_ref().and_then(|c| c.lint.as_ref()))
        {
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

        self.load_worms(&root);
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
        match pool_with(root, None, &mut fmt, crate::worm::registry::Fetch::Never) {
            Ok(pool) => {
                self.fmt = fmt;
                self.worm_stamp = stamp_of(&pool);
                self.worms = pool;
            }

            Err(_) => self.worms = no_worms(),
        }
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
