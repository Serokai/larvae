/*!
The paths that a config tells larvae to skip.

`[fmt]`, `[lint]`, and the file of selene all use the same form: a list of
globs relative to the project root. So the match logic lives here once and
not three times.

A match on any directory in the path counts. So a directory name excludes the
content under it, with or without a wildcard at the end of the pattern. The
user does not have to remember one specific spelling.

Excludes do not cover a file that the user named on the command line. A name
is an instruction from the user. A formatter that does nothing to a named
file, without a message, is worse than one that formats an excluded file.
Excludes apply only to the files that a walk finds.
*/

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use globset::{Glob, GlobSet, GlobSetBuilder};

/// One list as a set, or None for the empty list that costs nothing
fn set_of(patterns: &[String], what: &str) -> Result<Option<GlobSet>> {
    if patterns.is_empty() {
        return Ok(None);
    }

    let mut builder = GlobSetBuilder::new();

    for pattern in patterns {
        builder.add(
            Glob::new(pattern).with_context(|| format!("{what} \"{pattern}\" is not a glob"))?,
        );
    }

    Ok(Some(builder.build()?))
}

/// True when the set matches the path or any directory on the way to it
fn hits(set: &Option<GlobSet>, rel: &Path) -> bool {
    let Some(set) = set else {
        return false;
    };

    rel.ancestors()
        .filter(|a| !a.as_os_str().is_empty())
        .any(|a| set.is_match(a))
}

#[derive(Debug, Clone, Default)]
pub struct Excludes {
    root: PathBuf,
    /// None when the config excludes nothing; this is the usual case and skips the work
    set: Option<GlobSet>,
    /// The include list of the same area. A match comes back in over every exclude.
    keep: Option<GlobSet>,
    /// The root level exclude, which every area inherits
    root_set: Option<GlobSet>,
    /// The root level include. A match cancels the root exclude, and only that.
    root_keep: Option<GlobSet>,
}

impl Excludes {
    pub fn new(root: &Path, patterns: &[String]) -> Result<Self> {
        Self::layered(root, &[], patterns, &[], &[])
    }

    /*
    The four lists that decide one file, most specific first.

    1. The include of the area wins over every exclude. So a project excludes
       a tree at the root and one area still reads one file inside it.
    2. The exclude of the area removes the file for that area alone.
    3. The root include cancels the root exclude, and only that. It does not
       cancel the exclude of an area, because the local list is more specific
       than the global one.
    4. The root exclude removes the file for every area.

    Empty lists cost nothing and change nothing, so a config without the new
    keys behaves exactly as before.
    */
    pub fn layered(
        root: &Path,
        include: &[String],
        exclude: &[String],
        root_include: &[String],
        root_exclude: &[String],
    ) -> Result<Self> {
        Ok(Self {
            root: root.to_path_buf(),
            set: set_of(exclude, "exclude")?,
            keep: set_of(include, "include")?,
            root_set: set_of(root_exclude, "the root exclude")?,
            root_keep: set_of(root_include, "the root include")?,
        })
    }

    /// True when the project tells larvae to skip this path
    pub fn skips(&self, path: &Path) -> bool {
        let rel = path.strip_prefix(&self.root).unwrap_or(path);

        if hits(&self.keep, rel) {
            return false;
        }

        if hits(&self.set, rel) {
            return true;
        }

        if hits(&self.root_keep, rel) {
            return false;
        }

        hits(&self.root_set, rel)
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

    /// A directory name must be enough; users forget the /** part.
    #[test]
    fn naming_a_directory_excludes_what_is_under_it() {
        let e = excludes(&["Packages", "src/vendor"]);

        assert!(e.skips(Path::new("/project/Packages/signal/init.luau")));
        assert!(e.skips(Path::new("/project/src/vendor/a.luau")));
        assert!(!e.skips(Path::new("/project/src/vendored.luau")));
    }

    /// A path outside the root matches as written and does not pass without a
    /// check. The alternative is a format of an excluded file.
    #[test]
    fn a_path_outside_the_root_still_matches_on_its_own_terms() {
        assert!(excludes(&["**/vendor/**"]).skips(Path::new("/elsewhere/vendor/a.luau")));
    }

    #[test]
    fn a_pattern_that_is_not_a_glob_is_an_error() {
        assert!(Excludes::new(Path::new("/project"), &["a[".to_string()]).is_err());
    }

    fn layered(include: &[&str], exclude: &[&str], root_in: &[&str], root_ex: &[&str]) -> Excludes {
        let own = |list: &[&str]| list.iter().map(|p| p.to_string()).collect::<Vec<_>>();

        Excludes::layered(
            Path::new("/project"),
            &own(include),
            &own(exclude),
            &own(root_in),
            &own(root_ex),
        )
        .unwrap()
    }

    #[test]
    fn the_root_exclude_skips_for_every_area() {
        let e = layered(&[], &[], &[], &["gen/**"]);

        assert!(e.skips(Path::new("/project/gen/out.luau")));
        assert!(!e.skips(Path::new("/project/src/a.luau")));
    }

    #[test]
    fn the_root_include_cancels_the_root_exclude() {
        let e = layered(&[], &[], &["gen/keep.luau"], &["gen"]);

        assert!(!e.skips(Path::new("/project/gen/keep.luau")));
        assert!(e.skips(Path::new("/project/gen/out.luau")));
    }

    /// The list of the area is more specific than the root list, so the area
    /// exclude wins over the root include.
    #[test]
    fn the_area_exclude_wins_over_the_root_include() {
        let e = layered(&[], &["gen/keep.luau"], &["gen/keep.luau"], &["gen"]);

        assert!(e.skips(Path::new("/project/gen/keep.luau")));
    }

    #[test]
    fn the_area_include_wins_over_every_exclude() {
        let e = layered(&["gen/keep.luau"], &["gen"], &[], &["gen"]);

        assert!(!e.skips(Path::new("/project/gen/keep.luau")));
        assert!(e.skips(Path::new("/project/gen/out.luau")));
    }

    #[test]
    fn empty_root_lists_change_nothing() {
        let with = layered(&[], &["Packages"], &[], &[]);
        let without = excludes(&["Packages"]);

        for path in ["/project/Packages/a.luau", "/project/src/a.luau"] {
            assert_eq!(with.skips(Path::new(path)), without.skips(Path::new(path)));
        }
    }
}
