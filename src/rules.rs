//! Builtin token level rules, the AST rule engine arrives with the parser

use crate::syntax::lexer::{Tok, TokKind};
use crate::syntax::scan::RequireSite;

/*
const_requires, turn `local X = require(...)` into `const X = require(...)`
so single assignment requires get Luau's const treatment, conservative on
purpose, multi bindings and annotated locals are left alone
*/
pub fn const_requires(
    src: &str,
    toks: &[Tok],
    sites: &[RequireSite],
    replacements: &mut Vec<(u32, u32, String)>,
) {
    for site in sites {
        let i = site.require_idx;
        // pattern looking backward, local <name> = require
        if i < 3 {
            continue;
        }
        let (eq, name, local) = (&toks[i - 1], &toks[i - 2], &toks[i - 3]);
        if eq.kind == TokKind::Symbol
            && eq.text(src) == "="
            && name.kind == TokKind::Ident
            && local.kind == TokKind::Ident
            && local.text(src) == "local"
        {
            replacements.push((local.start, local.end, "const".to_string()));
        }
    }
}

/*
remove_comments, delete comment spans while keeping their newlines so
retain-lines output stays on the same line numbers, `except` patterns keep
matching comments, Luau directives like --!strict are kept by default
*/
pub fn remove_comments(
    src: &str,
    comments: &[(u32, u32)],
    except: &[regex::Regex],
    replacements: &mut Vec<(u32, u32, String)>,
) {
    let bytes = src.as_bytes();
    for &(start, end) in comments {
        let text = &src[start as usize..end as usize];
        if except.iter().any(|re| re.is_match(text)) {
            continue;
        }
        // eat the horizontal space in front so no trailing blanks are left
        let mut from = start as usize;
        while from > 0 && matches!(bytes[from - 1], b' ' | b'\t') {
            from -= 1;
        }
        let newlines = text.bytes().filter(|&b| b == b'\n').count();
        replacements.push((from as u32, end, "\n".repeat(newlines)));
    }
}

/*
add_luau_directive, make sure every file opts into a Luau mode, an existing
directive of the same kind is left alone and so is a different mode, an
explicit choice in the source always wins
*/
pub fn add_luau_directive(src: &str, directive: &str) -> Option<(u32, u32, String)> {
    let wanted = format!("--!{directive}");
    for line in src.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if let Some(rest) = line.strip_prefix("--!") {
            // same family, ex: strict vs nonstrict, respect what is there
            let head = |s: &str| s.split_whitespace().next().unwrap_or("").to_string();
            if line == wanted || mode_family(&head(rest)) == mode_family(&head(directive)) {
                return None;
            }
            continue;
        }
        if line.starts_with("--") {
            continue;
        }
        break;
    }
    Some((0, 0, format!("{wanted}\n")))
}

/// Directives that contradict each other, one per family
fn mode_family(word: &str) -> &'static str {
    match word {
        "strict" | "nonstrict" | "nocheck" => "typecheck",
        "native" => "native",
        "optimize" => "optimize",
        _ => "other",
    }
}

/// append_text_comment, add a comment at the start or end of the file
pub fn append_text_comment(src: &str, text: &str, at_start: bool) -> Option<(u32, u32, String)> {
    let comment: String = text
        .lines()
        .map(|line| format!("-- {line}\n"))
        .collect::<Vec<_>>()
        .concat();
    if at_start {
        Some((0, 0, comment))
    } else {
        let end = src.len() as u32;
        let lead = if src.ends_with('\n') { "" } else { "\n" };
        Some((end, end, format!("{lead}{comment}")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::syntax::{lexer, scan};

    fn apply(src: &str) -> String {
        let toks = lexer::lex(src).unwrap().toks;
        let scanned = scan::scan(src, &toks);
        let mut reps = Vec::new();
        const_requires(src, &toks, &scanned.sites, &mut reps);
        reps.sort_by_key(|r| r.0);
        let mut out = String::new();
        let mut cursor = 0usize;
        for (s, e, new) in reps {
            out.push_str(&src[cursor..s as usize]);
            out.push_str(&new);
            cursor = e as usize;
        }
        out.push_str(&src[cursor..]);
        out
    }

    #[test]
    fn converts_simple_require_locals() {
        assert_eq!(
            apply("local Signal = require(\"./signal\")"),
            "const Signal = require(\"./signal\")"
        );
        assert_eq!(
            apply("local a = 1\nlocal S = require(\"@pkg/s\")\n"),
            "local a = 1\nconst S = require(\"@pkg/s\")\n"
        );
    }

    #[test]
    fn leaves_tricky_forms_alone() {
        // multi binding
        assert_eq!(
            apply("local a, b = require(\"./x\"), 2"),
            "local a, b = require(\"./x\"), 2"
        );
        // type annotation
        assert_eq!(
            apply("local x: Foo = require(\"./x\")"),
            "local x: Foo = require(\"./x\")"
        );
        // not a local at all
        assert_eq!(apply("x = require(\"./x\")"), "x = require(\"./x\")");
        // dynamic require has no site
        assert_eq!(apply("local x = require(p)"), "local x = require(p)");
    }

    #[test]
    fn remove_comments_keeps_line_count_and_directives() {
        let src = "--!strict\nlocal a = 1 -- trailing\n--[[ block\nspans lines ]]\nreturn a\n";
        let lexed = lexer::lex(src).unwrap();
        let except = vec![regex::Regex::new("^--!").unwrap()];
        let mut reps = Vec::new();
        remove_comments(src, &lexed.comments, &except, &mut reps);
        reps.sort_by_key(|r| r.0);
        let mut out = String::new();
        let mut cursor = 0usize;
        for (s, e, new) in reps {
            out.push_str(&src[cursor..s as usize]);
            out.push_str(&new);
            cursor = e as usize;
        }
        out.push_str(&src[cursor..]);
        // directive kept, others gone, line count unchanged
        assert!(out.starts_with("--!strict"));
        assert!(!out.contains("trailing"));
        assert!(!out.contains("spans lines"));
        assert_eq!(src.lines().count(), out.lines().count());
    }

    #[test]
    fn luau_directive_respects_the_source() {
        assert_eq!(
            add_luau_directive("return 1\n", "strict").unwrap().2,
            "--!strict\n"
        );
        // already there
        assert!(add_luau_directive("--!strict\nreturn 1\n", "strict").is_none());
        // a different typecheck mode is an explicit choice
        assert!(add_luau_directive("--!nonstrict\nreturn 1\n", "strict").is_none());
        // an unrelated directive does not block it
        assert_eq!(
            add_luau_directive("--!native\nreturn 1\n", "strict")
                .unwrap()
                .2,
            "--!strict\n"
        );
        // plain leading comments do not block it either
        assert_eq!(
            add_luau_directive("-- header\nreturn 1\n", "strict")
                .unwrap()
                .2,
            "--!strict\n"
        );
    }

    #[test]
    fn append_comment_at_both_ends() {
        let (s, e, text) = append_text_comment("return 1\n", "generated", true).unwrap();
        assert_eq!((s, e), (0, 0));
        assert_eq!(text, "-- generated\n");

        let src = "return 1";
        let (s, e, text) = append_text_comment(src, "two\nlines", false).unwrap();
        assert_eq!((s, e), (8, 8));
        assert_eq!(text, "\n-- two\n-- lines\n");
    }
}
