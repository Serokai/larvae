/*!
The paths a config asked us to leave alone.

`[fmt]`, `[lint]` and selene's own file all spell this the same way, a list of
globs relative to the project root, so the matching lives here once rather than
three times over.

A match on any directory along the way counts, so naming a directory excludes
what is under it whether or not the pattern ends in a wildcard. Nobody should
have to remember which spelling this tool wanted.

What this does not cover is a file somebody named on the command line. Naming a
file is saying you meant it, and a formatter that silently does nothing to a
file you pointed it at is worse than one that formats a file you had excluded.
Excludes are for what a walk finds on its own.
*/

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use globset::{Glob, GlobSet, GlobSetBuilder};

#[derive(Debug, Clone, Default)]
pub struct Excludes {
    root: PathBuf,
    /// None when nothing was excluded, which is the usual case and skips the work
    set: Option<GlobSet>,
}

impl Excludes {
    pub fn new(root: &Path, patterns: &[String]) -> Result<Self> {
        if patterns.is_empty() {
            return Ok(Self::default());
        }

        let mut builder = GlobSetBuilder::new();

        for pattern in patterns {
            builder.add(
                Glob::new(pattern)
                    .with_context(|| format!("exclude \"{pattern}\" is not a glob"))?,
            );
        }

        Ok(Self {
            root: root.to_path_buf(),
            set: Some(builder.build()?),
        })
    }

    /// Whether this path is one the project asked us to pass over
    pub fn skips(&self, path: &Path) -> bool {
        let Some(set) = &self.set else {
            return false;
        };

        let rel = path.strip_prefix(&self.root).unwrap_or(path);

        rel.ancestors()
            .filter(|a| !a.as_os_str().is_empty())
            .any(|a| set.is_match(a))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn excludes(patterns: &[&str]) -> Excludes {
        let owned: Vec<String> = patterns.iter().map(|p| p.to_string()).collect();

        Excludes::new(Path::new("/project"), &owned).unwrap()
    }

    #[test]
    fn nothing_is_excluded_by_default() {
        assert!(!Excludes::default().skips(Path::new("/project/src/init.luau")));
        assert!(!excludes(&[]).skips(Path::new("/project/src/init.luau")));
    }

    #[test]
    fn a_glob_matches_the_path_below_the_root() {
        let e = excludes(&["Packages/**", "**/*.spec.luau"]);

        assert!(e.skips(Path::new("/project/Packages/signal/init.luau")));
        assert!(e.skips(Path::new("/project/src/thing.spec.luau")));
        assert!(!e.skips(Path::new("/project/src/thing.luau")));
    }

    /// Naming the directory has to be enough, the /** is the part people forget
    #[test]
    fn naming_a_directory_excludes_what_is_under_it() {
        let e = excludes(&["Packages", "src/vendor"]);

        assert!(e.skips(Path::new("/project/Packages/signal/init.luau")));
        assert!(e.skips(Path::new("/project/src/vendor/a.luau")));
        assert!(!e.skips(Path::new("/project/src/vendored.luau")));
    }

    /// A path from somewhere else entirely is matched as written rather than
    /// silently passing, since the alternative is formatting what was excluded
    #[test]
    fn a_path_outside_the_root_still_matches_on_its_own_terms() {
        assert!(excludes(&["**/vendor/**"]).skips(Path::new("/elsewhere/vendor/a.luau")));
    }

    #[test]
    fn a_pattern_that_is_not_a_glob_is_an_error() {
        assert!(Excludes::new(Path::new("/project"), &["a[".to_string()]).is_err());
    }
}
