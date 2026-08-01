/*!
Require resolution and rewrite emission, input follows the Luau RFCs and
output is the configured target, bad aliases and realm violations always
error, missing targets warn unless strict
*/

use std::collections::HashMap;
use std::collections::HashSet;
use std::path::{Path, PathBuf};

use crate::config::{IndexingStyle, Target};
use crate::diag::Diag;
use crate::project::luaurc::LuaurcIndex;
use crate::requires::datamodel::{
    DmPath, MountTable, Realm, ScriptKind, parse_game_path, script_instance_name, script_kind,
};

pub struct Resolver<'a> {
    /// Absolute project root
    pub root: &'a Path,
    /// `coldluau.toml` aliases, lowercased keys
    pub toml_aliases: &'a HashMap<String, String>,
    pub luaurc: &'a LuaurcIndex,
    pub mounts: &'a MountTable,
    pub target: Target,
    /// Child indexing for the roblox-instance target
    pub style: IndexingStyle,
    /// Quote char for generated string literals
    pub quote: char,
    pub strict: bool,
}

/// Per-file context, computed once
pub struct FileCtx<'a> {
    /// Absolute path of the file being processed
    pub path: &'a Path,
    pub dir: PathBuf,
    pub is_init: bool,
    pub kind: ScriptKind,
    pub dm: Option<DmPath>,
}

impl<'a> FileCtx<'a> {
    pub fn new(path: &'a Path, mounts: &MountTable) -> Self {
        let file_name = path.file_name().and_then(|s| s.to_str()).unwrap_or("");
        let is_init = script_instance_name(file_name).is_none();
        let dir = path.parent().unwrap_or(Path::new("")).to_owned();

        Self {
            path,
            dm: mounts.dm_of(path),
            is_init,
            kind: script_kind(file_name),
            dir,
        }
    }

    /// Directory ./ resolves against, init files resolve from their dir's parent
    fn dot_base(&self) -> &Path {
        if self.is_init {
            self.dir.parent().unwrap_or(&self.dir)
        } else {
            &self.dir
        }
    }

    /// Effective runtime realm of this file
    fn realm(&self) -> Option<Realm> {
        match self.kind {
            ScriptKind::Server => Some(Realm::ServerOnly),

            ScriptKind::Client => Some(Realm::StarterClone),

            ScriptKind::Module => self.dm.as_ref().map(|d| d.realm()),
        }
    }
}

pub enum Rewrite {
    Keep,
    /// Replace the require spec (the text between the quotes)
    Replace(String),
    /// Replace the whole argument with an instance expression
    Expr(String),
}

/// The filesystem node a require resolved to
#[derive(Debug, PartialEq, Eq)]
enum ModuleNode {
    /// A plain module file (`foo.luau`)
    File(PathBuf),
    /// A directory module (`foo/init.luau` exists), the *directory* is the node
    Dir(PathBuf),
}

impl ModuleNode {
    fn dm_key_path(&self) -> &Path {
        match self {
            ModuleNode::File(p) | ModuleNode::Dir(p) => p,
        }
    }
}

impl<'a> Resolver<'a> {
    /// Resolve one require spec, returns the rewrite decision and pushes diagnostics
    pub fn resolve(
        &self,
        ctx: &FileCtx,
        spec: &str,
        src: &str,
        offset: usize,
        diags: &mut Vec<Diag>,
    ) -> Rewrite {
        if let Some(rest) = strip_alias(spec, "self") {
            return self.resolve_self(ctx, spec, rest, src, offset, diags);
        }

        if let Some(rest) = strip_alias(spec, "game") {
            return self.resolve_game_passthrough(ctx, rest, src, offset, diags);
        }

        if let Some(alias_rest) = spec.strip_prefix('@') {
            return self.resolve_alias(ctx, spec, alias_rest, src, offset, diags);
        }

        if spec.starts_with("./") || spec.starts_with("../") {
            let base = ctx.dot_base().to_owned();
            let Some(target_base) = normalize_join(&base, spec) else {
                diags.push(
                    Diag::error(
                        ctx.path,
                        format!("require(\"{spec}\") escapes the filesystem root"),
                    )
                    .at(src, offset),
                );

                return Rewrite::Keep;
            };

            return self.emit_fs(ctx, spec, &target_base, src, offset, diags);
        }

        diags.push(
            Diag::error(
                ctx.path,
                format!(
                    "require(\"{spec}\") is not RFC-valid: paths must start with ./, ../, or @alias"
                ),
            )
            .at(src, offset)
            .with_help("write it as an explicitly relative path or define an alias"),
        );

        Rewrite::Keep
    }

    fn resolve_self(
        &self,
        ctx: &FileCtx,
        spec: &str,
        rest: &str,
        src: &str,
        offset: usize,
        diags: &mut Vec<Diag>,
    ) -> Rewrite {
        if !ctx.is_init {
            diags.push(
                Diag::error(ctx.path, format!("require(\"{spec}\"): @self is only valid inside an init module (a module-with-children)"))
                    .at(src, offset),
            );
        }

        // instance target resolves @self on disk like any other require
        if self.target == Target::RobloxInstance {
            let Some(target_base) = normalize_join(&ctx.dir, rest) else {
                diags.push(
                    Diag::error(
                        ctx.path,
                        format!("require(\"{spec}\") escapes the filesystem root"),
                    )
                    .at(src, offset),
                );

                return Rewrite::Keep;
            };

            return self.emit_fs(ctx, spec, &target_base, src, offset, diags);
        }

        // @self is natively valid for the other targets, pass through
        Rewrite::Keep
    }

    /// @game input is already native, validate it, instance target converts it
    fn resolve_game_passthrough(
        &self,
        ctx: &FileCtx,
        rest: &str,
        src: &str,
        offset: usize,
        diags: &mut Vec<Diag>,
    ) -> Rewrite {
        if self.target == Target::Path {
            diags.push(
                Diag::warning(ctx.path, format!("require(\"@game/{rest}\") cannot be converted for the path target; leaving it alone"))
                    .at(src, offset),
            );

            return Rewrite::Keep;
        }

        let segments: Vec<String> = rest
            .split('/')
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .collect();
        if let Some(service) = segments.first() {
            self.check_container_rules(ctx, service, src, offset, diags);
        }

        if self.target == Target::RobloxInstance {
            if segments.is_empty() {
                diags.push(
                    Diag::error(ctx.path, "require(\"@game\") has no path".to_string())
                        .at(src, offset),
                );

                return Rewrite::Keep;
            }

            return Rewrite::Expr(absolute_instance(self.style, self.quote, &segments));
        }

        Rewrite::Keep
    }

    fn resolve_alias(
        &self,
        ctx: &FileCtx,
        spec: &str,
        alias_rest: &str,
        src: &str,
        offset: usize,
        diags: &mut Vec<Diag>,
    ) -> Rewrite {
        let (name, rest) = split_alias(alias_rest);
        let mut name = name.to_lowercase();
        let mut rest = rest.to_string();
        let mut seen: HashSet<String> = HashSet::new();

        loop {
            if !seen.insert(name.clone()) {
                diags.push(
                    Diag::error(
                        ctx.path,
                        format!("require(\"{spec}\"): alias cycle detected through @{name}"),
                    )
                    .at(src, offset),
                );

                return Rewrite::Keep;
            }

            // coldluau.toml wins per key, then .luaurc upward walk
            let (value, base_dir): (&str, &Path) = if let Some(v) = self.toml_aliases.get(&name) {
                (v.as_str(), self.root)
            } else if let Some((v, dir)) = self.luaurc.lookup(&ctx.dir, &name) {
                (v, dir)
            } else {
                diags.push(
                    Diag::error(
                        ctx.path,
                        format!("require(\"{spec}\"): unknown alias @{name}"),
                    )
                    .at(src, offset)
                    .with_help("define it under [aliases] in coldluau.toml or in a .luaurc"),
                );

                return Rewrite::Keep;
            };

            // DataModel-valued alias, textual expansion
            if let Some(dm_base) = parse_game_path(value) {
                if self.target == Target::Path {
                    diags.push(
                        Diag::error(ctx.path, format!("require(\"{spec}\"): alias @{name} points at the DataModel ({value}), which the path target cannot express"))
                            .at(src, offset),
                    );

                    return Rewrite::Keep;
                }

                let mut segments = dm_base;
                segments.extend(
                    rest.split('/')
                        .filter(|s| !s.is_empty())
                        .map(str::to_string),
                );

                if segments.is_empty() {
                    diags.push(
                        Diag::error(
                            ctx.path,
                            format!(
                                "require(\"{spec}\"): expansion of @{name} produced an empty path"
                            ),
                        )
                        .at(src, offset),
                    );

                    return Rewrite::Keep;
                }

                self.check_container_rules(ctx, &segments[0], src, offset, diags);
                self.validate_dm_target_on_disk(ctx, spec, &segments, src, offset, diags);

                if self.target == Target::RobloxInstance {
                    return Rewrite::Expr(absolute_instance(self.style, self.quote, &segments));
                }

                return Rewrite::Replace(format!("@game/{}", segments.join("/")));
            }

            // Alias-to-alias chain (ex: a = "@b/sub")
            if let Some(chained) = value.strip_prefix('@') {
                let (next_name, value_rest) = split_alias(chained);

                if next_name.eq_ignore_ascii_case("game") {
                    unreachable!("handled by parse_game_path");
                }

                name = next_name.to_lowercase();
                rest = join_specs(value_rest, &rest);
                continue;
            }

            // Filesystem-valued alias, relative to wherever it was defined
            let joined = if rest.is_empty() {
                value.to_string()
            } else {
                join_specs(value, &rest)
            };

            let Some(target_base) = normalize_join(base_dir, &joined) else {
                diags.push(
                    Diag::error(ctx.path, format!("require(\"{spec}\"): alias @{name} resolves outside the filesystem root"))
                        .at(src, offset),
                );

                return Rewrite::Keep;
            };

            return self.emit_fs(ctx, spec, &target_base, src, offset, diags);
        }
    }

    /// Resolve a filesystem module base (extensionless) and emit for the target
    fn emit_fs(
        &self,
        ctx: &FileCtx,
        spec: &str,
        target_base: &Path,
        src: &str,
        offset: usize,
        diags: &mut Vec<Diag>,
    ) -> Rewrite {
        if has_module_extension(spec) {
            diags.push(
                Diag::error(
                    ctx.path,
                    format!(
                        "require(\"{spec}\"): require paths are extensionless per the Luau RFC"
                    ),
                )
                .at(src, offset),
            );

            return Rewrite::Keep;
        }

        let node = match resolve_module(target_base) {
            Ok(Some(node)) => node,

            Ok(None) => {
                let d = Diag::warning(
                    ctx.path,
                    format!(
                        "require(\"{spec}\"): no module found at {}",
                        crate::ui::rel(target_base)
                    ),
                )
                .at(src, offset);
                diags.push(if self.strict {
                    Diag {
                        severity: crate::diag::Severity::Error,
                        ..d
                    }
                } else {
                    d
                });

                return Rewrite::Keep;
            }

            Err(msg) => {
                diags.push(
                    Diag::error(ctx.path, format!("require(\"{spec}\"): {msg}")).at(src, offset),
                );

                return Rewrite::Keep;
            }
        };

        let out = match self.target {
            Target::Path => self.emit_path_target(ctx, &node),

            Target::RobloxString => {
                match self.emit_roblox_string(ctx, spec, &node, src, offset, diags) {
                    Some(s) => s,

                    None => return Rewrite::Keep,
                }
            }

            Target::RobloxInstance => {
                return match self.emit_roblox_instance(ctx, spec, &node, src, offset, diags) {
                    Some(expr) => Rewrite::Expr(expr),

                    None => Rewrite::Keep,
                };
            }
        };

        if out == spec {
            Rewrite::Keep
        } else {
            Rewrite::Replace(out)
        }
    }

    fn emit_path_target(&self, ctx: &FileCtx, node: &ModuleNode) -> String {
        let target = match node {
            ModuleNode::Dir(d) => d.clone(),

            ModuleNode::File(f) => f.with_extension(""),
        };

        fs_relative(ctx.dot_base(), &target)
    }

    /// Map both ends into the DataModel and run the shared container and realm checks
    fn check_dm(
        &self,
        ctx: &FileCtx,
        spec: &str,
        node: &ModuleNode,
        src: &str,
        offset: usize,
        diags: &mut Vec<Diag>,
    ) -> Option<(DmPath, DmPath)> {
        if self.mounts.is_empty() {
            diags.push(
                Diag::error(ctx.path, format!("require(\"{spec}\") needs a filesystem-to-DataModel mapping, but none is configured"))
                    .at(src, offset)
                    .with_help("add a Rojo project file (default.project.json) or [requires.mounts] to coldluau.toml"),
            );

            return None;
        }

        let Some(target_dm) = self.mounts.dm_of(node.dm_key_path()) else {
            diags.push(
                Diag::error(
                    ctx.path,
                    format!(
                        "require(\"{spec}\"): no mount covers {}",
                        crate::ui::rel(node.dm_key_path())
                    ),
                )
                .at(src, offset)
                .with_help("add it to [requires.mounts] or mount it in the Rojo project file"),
            );

            return None;
        };

        let Some(req_dm) = ctx.dm.clone() else {
            diags.push(
                Diag::error(ctx.path, format!("this file is not covered by any mount, so require(\"{spec}\") cannot be rewritten"))
                    .at(src, offset),
            );

            return None;
        };

        // Container/realm checks (plan §3.3)
        let target_realm = target_dm.realm();
        let req_realm = ctx.realm();

        if target_realm == Realm::ServerOnly {
            match req_realm {
                Some(Realm::StarterClone) => {
                    diags.push(
                        Diag::error(ctx.path, format!("require(\"{spec}\"): client code cannot require {} - {} does not replicate to clients", spec, target_dm.service()))
                            .at(src, offset)
                            .with_help("move the module to ReplicatedStorage"),
                    );

                    return None;
                }

                Some(Realm::Shared) | None => {
                    diags.push(
                        Diag::warning(ctx.path, format!("require(\"{spec}\") targets {} from shared code; this breaks if the requirer ever runs on the client", target_dm.service()))
                            .at(src, offset),
                    );
                }

                Some(Realm::ServerOnly) => {}
            }
        }

        if target_realm == Realm::StarterClone {
            // absolute @game into a Starter container hits the template, only same container relatives work
            if req_dm.service() != target_dm.service() {
                diags.push(
                    Diag::error(ctx.path, format!("require(\"{spec}\") targets {} from outside it; Starter containers run as clones, so this would resolve to the template", target_dm.service()))
                        .at(src, offset)
                        .with_help("move the module to ReplicatedStorage"),
                );

                return None;
            }
        }

        // Self require?
        if req_dm.segments == target_dm.segments {
            diags.push(
                Diag::error(
                    ctx.path,
                    format!("require(\"{spec}\"): module requires itself"),
                )
                .at(src, offset),
            );

            return None;
        }

        Some((req_dm, target_dm))
    }

    fn emit_roblox_string(
        &self,
        ctx: &FileCtx,
        spec: &str,
        node: &ModuleNode,
        src: &str,
        offset: usize,
        diags: &mut Vec<Diag>,
    ) -> Option<String> {
        let (req_dm, target_dm) = self.check_dm(ctx, spec, node, src, offset, diags)?;

        // Child of the requirer -> @self
        if target_dm.segments.len() > req_dm.segments.len()
            && target_dm.segments[..req_dm.segments.len()] == req_dm.segments[..]
        {
            let tail = target_dm.segments[req_dm.segments.len()..].join("/");

            return Some(format!("@self/{tail}"));
        }

        // relative within the same mount, absolute @game otherwise, Starter targets must be relative
        let same_mount = req_dm.mount == target_dm.mount;
        let base = &req_dm.segments[..req_dm.segments.len() - 1];

        let common = base
            .iter()
            .zip(&target_dm.segments)
            .take_while(|(a, b)| a == b)
            .count();
        let relative_ok = same_mount && common >= req_dm.mount_depth.min(target_dm.mount_depth);

        if relative_ok || target_dm.realm() == Realm::StarterClone {
            if !relative_ok {
                diags.push(
                    Diag::error(ctx.path, format!("require(\"{spec}\") cannot be expressed as a relative require within {}", target_dm.service()))
                        .at(src, offset),
                );

                return None;
            }

            let ups = base.len() - common;
            let mut parts: Vec<&str> = std::iter::repeat_n("..", ups).collect();

            parts.extend(target_dm.segments[common..].iter().map(String::as_str));

            return Some(if ups == 0 {
                format!("./{}", parts.join("/"))
            } else {
                parts.join("/")
            });
        }

        Some(target_dm.game_path())
    }

    /// Build an Instance expression like script.Parent:FindFirstChild("shared")
    fn emit_roblox_instance(
        &self,
        ctx: &FileCtx,
        spec: &str,
        node: &ModuleNode,
        src: &str,
        offset: usize,
        diags: &mut Vec<Diag>,
    ) -> Option<String> {
        let (req_dm, target_dm) = self.check_dm(ctx, spec, node, src, offset, diags)?;

        /*
        script relative chains survive Starter cloning, mandatory there and
        preferred in the same mount, everything else goes absolute
        */
        let same_mount = req_dm.mount == target_dm.mount;

        if same_mount || target_dm.realm() == Realm::StarterClone {
            let common = req_dm
                .segments
                .iter()
                .zip(&target_dm.segments)
                .take_while(|(a, b)| a == b)
                .count();
            let ups = req_dm.segments.len() - common;
            let mut expr = String::from("script");

            for _ in 0..ups {
                expr.push_str(".Parent");
            }

            push_downs(
                &mut expr,
                self.style,
                self.quote,
                &target_dm.segments[common..],
            );

            return Some(expr);
        }

        Some(absolute_instance(
            self.style,
            self.quote,
            &target_dm.segments,
        ))
    }

    /// Best effort check that a @game expansion maps back to a real file
    fn validate_dm_target_on_disk(
        &self,
        ctx: &FileCtx,
        spec: &str,
        segments: &[String],
        src: &str,
        offset: usize,
        diags: &mut Vec<Diag>,
    ) {
        for mount in self.mounts.mounts() {
            if segments.len() < mount.dm.len() || segments[..mount.dm.len()] != mount.dm[..] {
                continue;
            }

            let mut fs = mount.fs.clone();

            for seg in &segments[mount.dm.len()..] {
                fs.push(seg);
            }

            match resolve_module(&fs) {
                Ok(Some(_)) => return,

                _ => {
                    let d = Diag::warning(
                        ctx.path,
                        format!("require(\"{spec}\"): expansion targets @game/{} but no module exists at {}", segments.join("/"), crate::ui::rel(&fs)),
                    )
                    .at(src, offset);
                    diags.push(if self.strict {
                        Diag {
                            severity: crate::diag::Severity::Error,
                            ..d
                        }
                    } else {
                        d
                    });

                    return;
                }
            }
        }

        // No mount covers the alias target, nothing to validate against
    }

    fn check_container_rules(
        &self,
        ctx: &FileCtx,
        service: &str,
        src: &str,
        offset: usize,
        diags: &mut Vec<Diag>,
    ) {
        match crate::requires::datamodel::realm_of_container(service) {
            Realm::StarterClone => diags.push(
                Diag::error(ctx.path, format!("absolute require into {service}: Starter containers run as clones, so @game paths into them resolve to the template"))
                    .at(src, offset)
                    .with_help("use a relative require from within the container, or move the module to ReplicatedStorage"),
            ),
            Realm::ServerOnly => {
                if ctx.realm() == Some(Realm::StarterClone) {
                    diags.push(
                        Diag::error(ctx.path, format!("client code cannot require from {service}: it does not replicate to clients"))
                            .at(src, offset),
                    );
                }
            }

            Realm::Shared => {}
        }
    }
}

// --- instance expression building --------------------------------------------

/*
Absolute instance path from the root, property style uses plain dots,
method styles use GetService so service renames don't break them
*/
fn absolute_instance(style: IndexingStyle, quote: char, segments: &[String]) -> String {
    let mut expr = String::from("game");

    match style {
        IndexingStyle::Property => push_downs(&mut expr, style, quote, segments),

        IndexingStyle::FindFirstChild | IndexingStyle::WaitForChild => {
            expr.push_str(&format!(
                ":GetService({})",
                lua_quote(segments[0].as_str(), quote)
            ));
            push_downs(&mut expr, style, quote, &segments[1..]);
        }
    }

    expr
}

/// Append child-indexing steps in the chosen style
fn push_downs(expr: &mut String, style: IndexingStyle, quote: char, segments: &[String]) {
    for seg in segments {
        match style {
            IndexingStyle::Property => {
                if is_luau_ident(seg) {
                    expr.push('.');
                    expr.push_str(seg);
                } else {
                    expr.push_str(&format!("[{}]", lua_quote(seg, quote)));
                }
            }

            IndexingStyle::FindFirstChild => {
                expr.push_str(&format!(":FindFirstChild({})", lua_quote(seg, quote)));
            }

            IndexingStyle::WaitForChild => {
                expr.push_str(&format!(":WaitForChild({})", lua_quote(seg, quote)));
            }
        }
    }
}

/// Valid Luau identifier that isn't a keyword (usable after a dot)
fn is_luau_ident(name: &str) -> bool {
    const KEYWORDS: [&str; 21] = [
        "and", "break", "do", "else", "elseif", "end", "false", "for", "function", "if", "in",
        "local", "nil", "not", "or", "repeat", "return", "then", "true", "until", "while",
    ];
    let mut chars = name.chars();
    let Some(first) = chars.next() else {
        return false;
    };

    (first.is_ascii_alphabetic() || first == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
        && !KEYWORDS.contains(&name)
}

/// A quoted Luau string literal using the chosen quote char
pub fn lua_quote(name: &str, quote: char) -> String {
    let escaped = name
        .replace('\\', "\\\\")
        .replace(quote, &format!("\\{quote}"));
    format!("{quote}{escaped}{quote}")
}

// --- helpers -----------------------------------------------------------------

fn strip_alias<'s>(spec: &'s str, name: &str) -> Option<&'s str> {
    let body = spec.strip_prefix('@')?;

    if body.eq_ignore_ascii_case(name) {
        return Some("");
    }

    let rest = body.get(..name.len() + 1)?;

    if rest[..name.len()].eq_ignore_ascii_case(name) && rest.ends_with('/') {
        Some(&body[name.len() + 1..])
    } else {
        None
    }
}

fn split_alias(body: &str) -> (&str, &str) {
    match body.find('/') {
        Some(i) => (&body[..i], &body[i + 1..]),

        None => (body, ""),
    }
}

fn join_specs(a: &str, b: &str) -> String {
    match (a.is_empty(), b.is_empty()) {
        (true, _) => b.to_string(),

        (_, true) => a.to_string(),

        _ => format!("{}/{}", a.trim_end_matches('/'), b),
    }
}

fn has_module_extension(spec: &str) -> bool {
    spec.ends_with(".luau") || spec.ends_with(".lua")
}

/// Join a spec onto a base dir resolving dots, None when it escapes the root
fn normalize_join(base: &Path, spec: &str) -> Option<PathBuf> {
    let mut out = base.to_owned();

    for part in spec.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                if !out.pop() {
                    return None;
                }
            }

            seg => out.push(seg),
        }
    }

    Some(out)
}

/// RFC module resolution, file or dir with init, more than one match is ambiguous
fn resolve_module(base: &Path) -> Result<Option<ModuleNode>, String> {
    let mut found: Vec<ModuleNode> = Vec::new();

    for ext in ["luau", "lua"] {
        let candidate = base.with_extension(ext);

        if candidate.is_file() {
            found.push(ModuleNode::File(candidate));
        }
    }

    if base.is_dir() && (base.join("init.luau").is_file() || base.join("init.lua").is_file()) {
        found.push(ModuleNode::Dir(base.to_owned()));
    }

    match found.len() {
        0 => Ok(None),

        1 => Ok(Some(found.remove(0))),

        _ => Err(format!(
            "ambiguous module at {} (multiple of .luau/.lua/dir-with-init exist; the RFC makes this an error)",
            base.display()
        )),
    }
}

/// Relative path in require syntax, ./x/y or ../../x
fn fs_relative(from_dir: &Path, to: &Path) -> String {
    let from: Vec<_> = from_dir.components().collect();
    let to_comps: Vec<_> = to.components().collect();

    let common = from
        .iter()
        .zip(&to_comps)
        .take_while(|(a, b)| a == b)
        .count();
    let ups = from.len() - common;
    let mut parts: Vec<String> = std::iter::repeat_n("..".to_string(), ups).collect();

    parts.extend(
        to_comps[common..]
            .iter()
            .map(|c| c.as_os_str().to_string_lossy().into_owned()),
    );

    if ups == 0 {
        format!("./{}", parts.join("/"))
    } else {
        parts.join("/")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn instance_expressions() {
        let segs: Vec<String> = ["ReplicatedStorage", "shared", "for"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert_eq!(
            absolute_instance(IndexingStyle::Property, '"', &segs),
            "game.ReplicatedStorage.shared[\"for\"]"
        );
        assert_eq!(
            absolute_instance(IndexingStyle::FindFirstChild, '"', &segs),
            "game:GetService(\"ReplicatedStorage\"):FindFirstChild(\"shared\"):FindFirstChild(\"for\")"
        );
        assert_eq!(
            absolute_instance(IndexingStyle::WaitForChild, '"', &segs),
            "game:GetService(\"ReplicatedStorage\"):WaitForChild(\"shared\"):WaitForChild(\"for\")"
        );
    }

    #[test]
    fn ident_rules() {
        assert!(is_luau_ident("math"));
        assert!(is_luau_ident("_Foo2"));
        assert!(!is_luau_ident("for"));
        assert!(!is_luau_ident("2fast"));
        assert!(!is_luau_ident("has space"));
        assert!(!is_luau_ident(""));
    }

    #[test]
    fn spec_helpers() {
        assert_eq!(strip_alias("@self/x", "self"), Some("x"));
        assert_eq!(strip_alias("@Self", "self"), Some(""));
        assert_eq!(strip_alias("@selfish/x", "self"), None);
        assert_eq!(split_alias("pkg/a/b"), ("pkg", "a/b"));
        assert_eq!(split_alias("pkg"), ("pkg", ""));
        assert_eq!(join_specs("a/", "b"), "a/b");
    }

    #[test]
    fn normalize() {
        assert_eq!(
            normalize_join(Path::new("/a/b"), "../c/./d").unwrap(),
            PathBuf::from("/a/c/d")
        );
        assert!(normalize_join(Path::new("/"), "../..").is_none());
    }

    #[test]
    fn fs_relative_paths() {
        assert_eq!(fs_relative(Path::new("/a/b"), Path::new("/a/b/c")), "./c");
        assert_eq!(
            fs_relative(Path::new("/a/b"), Path::new("/a/x/y")),
            "../x/y"
        );
        assert_eq!(fs_relative(Path::new("/a/b/c"), Path::new("/a")), "../..");
    }
}
