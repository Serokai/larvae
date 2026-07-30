//! The pipeline, discover, then parallel lex/scan/resolve/splice, then atomic writes

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use anyhow::{Context, Result};
use globset::{Glob, GlobSet, GlobSetBuilder};
use rayon::prelude::*;
use walkdir::WalkDir;

use crate::cache::{Cache, EpochInputs, hash_bytes};
use crate::config::{Config, QuoteStyle};
use crate::diag::{self, Diag};
use crate::project::luaurc::LuaurcIndex;
use crate::project::rojo;
use crate::requires::datamodel::{Mount, MountTable, parse_game_path};
use crate::requires::resolve::{FileCtx, Resolver, Rewrite, lua_quote};
use crate::syntax::lexer;
use crate::syntax::scan;

#[derive(Debug, Default)]
pub struct Stats {
    pub files_processed: usize,
    pub files_copied: usize,
    pub files_cached: usize,
    pub files_pruned: usize,
    pub requires_rewritten: usize,
    pub requires_dynamic: usize,
}

pub struct Outcome {
    pub stats: Stats,
    pub diags: Vec<Diag>,
    pub build_project: Option<PathBuf>,
}

/// Per file transform options, resolved from the config once
struct FileOpts {
    quotes: QuoteStyle,
    /// Check mode parses too, so syntax errors surface before Studio sees them
    validate_syntax: bool,
    const_requires: bool,
    /// Compiled `except` patterns, Some means the rule is on
    remove_comments: Option<Vec<regex::Regex>>,
    /// Comment text and whether it goes at the start
    append_comment: Option<(String, bool)>,
    directive: Option<String>,
}

impl Outcome {
    pub fn has_errors(&self) -> bool {
        self.diags
            .iter()
            .any(|d| d.severity == diag::Severity::Error)
    }
}

pub fn run(root: &Path, config: &Config, write: bool) -> Result<Outcome> {
    let root = root
        .canonicalize()
        .with_context(|| format!("cannot resolve project root {}", root.display()))?;
    let input = root.join(&config.process.input);
    let output = root.join(&config.process.output);
    if !input.is_dir() {
        anyhow::bail!(
            "input directory {} does not exist (set [process].input in coldluau.toml)",
            crate::ui::rel(&input)
        );
    }

    let mut diags: Vec<Diag> = Vec::new();

    // --- Rojo project, auto mounts + (later) derived build project ---------
    let project = match rojo::find_project(&root, config.rojo.project.as_deref()) {
        Some(path) => match rojo::load(&path) {
            Ok(p) => Some(p),
            Err(e) => {
                diags.push(Diag::error(&path, format!("{e:#}")));
                None
            }
        },
        None => None,
    };

    // --- Mount table, config mounts win over project-derived ones ----------
    let mut mounts: Vec<Mount> = Vec::new();
    for (fs_rel, dm_value) in &config.requires.mounts {
        match parse_game_path(dm_value) {
            Some(dm) => mounts.push(Mount { fs: normalize(&root.join(fs_rel)), dm }),
            None => diags.push(Diag::error(
                Path::new("coldluau.toml"),
                format!("[requires.mounts] \"{fs_rel}\" = \"{dm_value}\": value must be an @game/... path"),
            )),
        }
    }
    if let Some(p) = &project {
        for m in rojo::mounts(p) {
            if !mounts.iter().any(|existing| existing.fs == m.fs) {
                mounts.push(m);
            }
        }
    }
    let mounts = MountTable::new(mounts);

    // --- .luaurc index ------------------------------------------------------
    let mut luaurc = LuaurcIndex::new(&root);
    let skip_dirs = [
        output.clone(),
        root.join(&config.process.cache_dir),
        root.join(".git"),
        root.join("target"),
    ];
    for entry in WalkDir::new(&root)
        .into_iter()
        .filter_entry(|e| !skip_dirs.iter().any(|s| e.path() == s))
        .filter_map(|e| e.ok())
    {
        if entry.file_type().is_file()
            && entry.file_name() == ".luaurc"
            && let Err(msg) = luaurc.add_file(entry.path())
        {
            diags.push(Diag::error(entry.path(), msg));
        }
    }

    // --- Discover input files ----------------------------------------------
    let include = build_globset(&config.process.include)?;
    let exclude = build_globset(&config.process.exclude)?;
    let mut to_process: Vec<PathBuf> = Vec::new();
    let mut to_copy: Vec<PathBuf> = Vec::new();
    for entry in WalkDir::new(&input).into_iter().filter_map(|e| e.ok()) {
        if !entry.file_type().is_file() {
            continue;
        }
        let rel = entry.path().strip_prefix(&input).unwrap();
        if exclude.is_match(rel) {
            continue;
        }
        if include.is_match(rel) {
            to_process.push(entry.path().to_owned());
        } else if entry.file_name() != ".luaurc" {
            to_copy.push(entry.path().to_owned());
        }
    }

    // --- Resolution epoch, everything require resolution reads --------------
    let mut epoch = EpochInputs::new();
    if let Some(p) = &project {
        epoch.add_file(&p.path);
    }
    let cfg_path = root.join("coldluau.toml");
    if cfg_path.exists() {
        epoch.add_file(&cfg_path);
    }
    for entry in WalkDir::new(&root)
        .into_iter()
        .filter_entry(|e| !skip_dirs.iter().any(|s| e.path() == s))
        .filter_map(|e| e.ok())
    {
        if entry.file_type().is_file() && entry.file_name() == ".luaurc" {
            epoch.add_file(entry.path());
        }
    }
    let mut all_paths: Vec<PathBuf> = to_process.iter().chain(to_copy.iter()).cloned().collect();
    epoch.add_paths(&mut all_paths);
    let epoch = epoch.finish();
    // caching only applies when writing, check must re-report every diagnostic
    let mut cache = Cache::load(
        &root.join(&config.process.cache_dir),
        epoch,
        config.process.cache && write,
    );

    let resolver = Resolver {
        root: &root,
        toml_aliases: &config.alias_map(),
        luaurc: &luaurc,
        mounts: &mounts,
        target: config.requires.target,
        style: config.requires.indexing_style.unwrap_or_default(),
        quote: config.process.quotes.char(),
        strict: config.requires.strict,
    };
    let remove_comments = match &config.rules.remove_comments {
        Some(rc) if rc.enabled() => {
            let mut pats = Vec::new();
            for p in rc.except() {
                match regex::Regex::new(&p) {
                    Ok(re) => pats.push(re),
                    Err(e) => anyhow::bail!("invalid remove_comments except pattern \"{p}\": {e}"),
                }
            }
            Some(pats)
        }
        _ => None,
    };
    let append_comment = match &config.rules.append_text_comment {
        Some(a) => {
            let text = match (&a.text, &a.file) {
                (Some(t), _) => t.clone(),
                (None, Some(f)) => std::fs::read_to_string(root.join(f))
                    .with_context(|| format!("append_text_comment file {}", crate::ui::rel(f)))?,
                _ => unreachable!("validated at load"),
            };
            Some((text, a.location == "start"))
        }
        None => None,
    };
    let opts = FileOpts {
        quotes: config.process.quotes,
        validate_syntax: !write,
        const_requires: config.rules.const_requires,
        remove_comments,
        append_comment,
        directive: config.rules.add_luau_directive.clone(),
    };

    // --- Parallel per file processing ---------------------------------------
    let shared_diags = Mutex::new(diags);
    let stats = Mutex::new(Stats::default());

    let fresh_hashes: Mutex<Vec<(String, u64)>> = Mutex::new(Vec::new());

    to_process.par_iter().for_each(|path| {
        let rel = path.strip_prefix(&input).unwrap();
        let rel_key = rel.to_string_lossy().into_owned();
        let source = match std::fs::read(path) {
            Ok(b) => b,
            Err(e) => {
                shared_diags
                    .lock()
                    .unwrap()
                    .push(Diag::error(path, format!("cannot read file: {e}")));
                return;
            }
        };
        let source_hash = hash_bytes(&source);
        if cache.is_fresh(&rel_key, source_hash, &output.join(rel)) {
            let mut s = stats.lock().unwrap();
            s.files_processed += 1;
            s.files_cached += 1;
            return;
        }

        let mut local_diags = Vec::new();
        let rewritten = process_file(
            path,
            &input,
            &output,
            &resolver,
            &opts,
            write,
            &mut local_diags,
        );
        let mut s = stats.lock().unwrap();
        s.files_processed += 1;
        if let Some((rewrites, dynamic)) = rewritten {
            s.requires_rewritten += rewrites;
            s.requires_dynamic += dynamic;
        }
        drop(s);
        // only a clean file earns a cache entry, errors must resurface
        if rewritten.is_some()
            && !local_diags
                .iter()
                .any(|d| d.severity == diag::Severity::Error)
        {
            fresh_hashes.lock().unwrap().push((rel_key, source_hash));
        }
        if !local_diags.is_empty() {
            shared_diags.lock().unwrap().extend(local_diags);
        }
    });

    if write {
        to_copy.par_iter().for_each(|path| {
            let rel = path.strip_prefix(&input).unwrap();
            let dest = output.join(rel);
            if let Err(e) = copy_atomic(path, &dest) {
                shared_diags
                    .lock()
                    .unwrap()
                    .push(Diag::error(path, format!("copy failed: {e:#}")));
            } else {
                stats.lock().unwrap().files_copied += 1;
            }
        });
    }

    let mut diags = shared_diags.into_inner().unwrap();
    let mut stats = stats.into_inner().unwrap();

    // --- Cache bookkeeping ---------------------------------------------------
    if write {
        let mut keep: HashSet<String> = HashSet::new();
        for (rel, hash) in fresh_hashes.into_inner().unwrap() {
            keep.insert(rel.clone());
            cache.record(rel, hash);
        }
        for path in &to_process {
            if let Ok(rel) = path.strip_prefix(&input) {
                keep.insert(rel.to_string_lossy().into_owned());
            }
        }
        cache.retain(&keep);
        if let Err(e) = cache.save() {
            diags.push(Diag::warning(
                Path::new(".coldluau"),
                format!("could not write the build cache: {e}"),
            ));
        }

        // --- Mirror deletions, stale output is worse than slow output --------
        let produced: HashSet<PathBuf> = to_process
            .iter()
            .chain(to_copy.iter())
            .filter_map(|p| p.strip_prefix(&input).ok().map(|r| output.join(r)))
            .collect();
        stats.files_pruned = prune_output(&output, &input, &root, &produced, &mut diags);
    }

    // --- Derived build project (only when writing and a place project exists)
    let mut build_project = None;
    if write && let Some(p) = &project {
        match rojo::write_build_project(
            p,
            &input,
            &output,
            &root.join(&config.process.cache_dir),
            config
                .rojo
                .build_project
                .as_deref()
                .map(|bp| root.join(bp))
                .as_deref(),
        ) {
            Ok((path, warnings)) => {
                for w in warnings {
                    diags.push(Diag::warning(&p.path, w));
                }
                build_project = Some(path);
            }
            Err(e) => diags.push(Diag::warning(&p.path, format!("{e:#}"))),
        }
    }

    diag::sort(&mut diags);
    Ok(Outcome {
        stats,
        diags,
        build_project,
    })
}

/// Returns (rewrites, dynamic_count) on success, None if the file was skipped
fn process_file(
    path: &Path,
    input: &Path,
    output: &Path,
    resolver: &Resolver,
    opts: &FileOpts,
    write: bool,
    diags: &mut Vec<Diag>,
) -> Option<(usize, usize)> {
    let src = match std::fs::read_to_string(path) {
        Ok(s) => s,
        Err(e) => {
            diags.push(Diag::error(
                path,
                format!("cannot read file (UTF-8 required): {e}"),
            ));
            return None;
        }
    };
    let lexed = match lexer::lex(&src) {
        Ok(t) => t,
        Err(e) => {
            diags.push(Diag::error(path, format!("lex error: {}", e.message)).at(&src, e.offset));
            return None;
        }
    };
    if opts.validate_syntax
        && let Err(e) = crate::syntax::parser::parse(&src, &lexed.toks)
    {
        diags.push(Diag::error(path, format!("syntax error, {}", e.message)).at(&src, e.offset));
    }
    let scanned = scan::scan(&src, &lexed.toks);
    let ctx = FileCtx::new(path, resolver.mounts);

    let quote = opts.quotes.char();
    let requote = opts.quotes != QuoteStyle::Preserve;
    let mut replacements: Vec<(u32, u32, String)> = Vec::new();
    for site in &scanned.sites {
        let spec = &src[site.inner_start as usize..site.inner_end as usize];
        match resolver.resolve(&ctx, spec, &src, site.at as usize, diags) {
            Rewrite::Keep => {
                // untouched requires still get the configured quote style
                if requote
                    && src.as_bytes()[site.tok_start as usize] != quote as u8
                    && !spec.contains(['"', '\'', '\\'])
                {
                    replacements.push((site.tok_start, site.tok_end, lua_quote(spec, quote)));
                }
            }
            Rewrite::Replace(new) => {
                if requote {
                    replacements.push((site.tok_start, site.tok_end, lua_quote(&new, quote)));
                } else {
                    replacements.push((site.inner_start, site.inner_end, new));
                }
            }
            // instance exprs replace the whole argument, parenless calls need wrapping parens
            Rewrite::Expr(expr) => {
                let expr = if site.has_parens {
                    expr
                } else {
                    format!("({expr})")
                };
                replacements.push((site.tok_start, site.tok_end, expr));
            }
        }
    }
    let rewrites = replacements.len();
    if opts.const_requires {
        crate::rules::const_requires(&src, &lexed.toks, &scanned.sites, &mut replacements);
    }
    if let Some(except) = &opts.remove_comments {
        crate::rules::remove_comments(&src, &lexed.comments, except, &mut replacements);
    }
    if let Some(directive) = &opts.directive
        && let Some(rep) = crate::rules::add_luau_directive(&src, directive)
    {
        replacements.push(rep);
    }
    if let Some((text, at_start)) = &opts.append_comment
        && let Some(rep) = crate::rules::append_text_comment(&src, text, *at_start)
    {
        replacements.push(rep);
    }
    replacements.sort_by_key(|r| (r.0, r.1));

    if write {
        let rel = path.strip_prefix(input).unwrap();
        let dest = output.join(rel);
        let out_src = splice(&src, &replacements);
        if let Err(e) = write_atomic(&dest, out_src.as_bytes()) {
            diags.push(Diag::error(path, format!("write failed: {e:#}")));
        }
    }
    Some((rewrites, scanned.dynamic.len()))
}

/// Replace byte ranges (sorted, non-overlapping) in `src`
fn splice(src: &str, replacements: &[(u32, u32, String)]) -> String {
    if replacements.is_empty() {
        return src.to_owned();
    }
    let extra: usize = replacements.iter().map(|(_, _, s)| s.len()).sum();
    let mut out = String::with_capacity(src.len() + extra);
    let mut cursor = 0usize;
    for (start, end, new) in replacements {
        // two rules touching the same bytes would corrupt the output, the
        // first one wins rather than producing a mangled file
        if (*start as usize) < cursor {
            continue;
        }
        out.push_str(&src[cursor..*start as usize]);
        out.push_str(new);
        cursor = *end as usize;
    }
    out.push_str(&src[cursor..]);
    out
}

fn write_atomic(dest: &Path, bytes: &[u8]) -> Result<()> {
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp = dest.with_extension("coldluau-tmp");
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(&tmp, dest)?;
    Ok(())
}

fn copy_atomic(from: &Path, dest: &Path) -> Result<()> {
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp = dest.with_extension("coldluau-tmp");
    std::fs::copy(from, &tmp)?;
    std::fs::rename(&tmp, dest)?;
    Ok(())
}

/*
Remove output files whose source is gone, guarded so it can never chew
through the project when output is misconfigured
*/
fn prune_output(
    output: &Path,
    input: &Path,
    root: &Path,
    produced: &HashSet<PathBuf>,
    diags: &mut Vec<Diag>,
) -> usize {
    if !output.is_dir() || output == input || output == root {
        return 0;
    }
    let mut removed = 0;
    for entry in WalkDir::new(output).into_iter().filter_map(|e| e.ok()) {
        if !entry.file_type().is_file() {
            continue;
        }
        if produced.contains(entry.path()) {
            continue;
        }
        match std::fs::remove_file(entry.path()) {
            Ok(()) => removed += 1,
            Err(e) => diags.push(Diag::warning(
                entry.path(),
                format!("stale output could not be removed: {e}"),
            )),
        }
    }
    removed
}

fn build_globset(patterns: &[String]) -> Result<GlobSet> {
    let mut b = GlobSetBuilder::new();
    for p in patterns {
        b.add(Glob::new(p).with_context(|| format!("invalid glob \"{p}\""))?);
    }
    Ok(b.build()?)
}

fn normalize(p: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in p.components() {
        match c {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                out.pop();
            }
            other => out.push(other),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splice_replaces_ranges() {
        let src = r#"require("./a") + require("./b")"#;
        let out = splice(
            src,
            &[(9, 12, "@game/RS/a".into()), (26, 29, "@game/RS/b".into())],
        );
        assert_eq!(out, r#"require("@game/RS/a") + require("@game/RS/b")"#);
    }
}
