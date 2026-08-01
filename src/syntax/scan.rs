//! Finds `require("...")` call sites in a token stream

use crate::syntax::lexer::{Tok, TokKind};

#[derive(Debug, Clone, Copy)]
pub struct RequireSite {
    /// Byte range of the string literal's content (between quotes/brackets)
    pub inner_start: u32,
    pub inner_end: u32,
    /// Whole string token with quotes, instance rewrites replace this span
    pub tok_start: u32,
    pub tok_end: u32,
    /// True for require("x"), parenless calls need the expression wrapped in parens
    pub has_parens: bool,
    /// Byte offset of the `require` identifier, for diagnostics
    pub at: u32,
    /// Index of the require ident in the token stream, for context checks
    pub require_idx: usize,
}

#[derive(Debug, Default)]
pub struct ScanResult {
    pub sites: Vec<RequireSite>,
    /// Offsets of dynamic requires, left untouched and counted by check
    pub dynamic: Vec<u32>,
}

pub fn scan(src: &str, toks: &[Tok]) -> ScanResult {
    let mut out = ScanResult::default();

    for (i, tok) in toks.iter().enumerate() {
        if tok.kind != TokKind::Ident || tok.text(src) != "require" {
            continue;
        }

        // `foo.require(...)` / `foo:require(...)` is not the global require
        if i > 0 && matches!(toks[i - 1].kind, TokKind::Dot | TokKind::Colon) {
            continue;
        }

        match toks.get(i + 1).map(|t| t.kind) {
            // require("path")
            Some(TokKind::LParen) => match (toks.get(i + 2), toks.get(i + 3)) {
                (
                    Some(&Tok {
                        kind:
                            TokKind::Str {
                                inner_start,
                                inner_end,
                            },

                        start,
                        end,
                    }),
                    Some(&Tok {
                        kind: TokKind::RParen,
                        ..
                    }),
                ) => out.sites.push(RequireSite {
                    inner_start,
                    inner_end,
                    tok_start: start,
                    tok_end: end,
                    has_parens: true,
                    at: tok.start,
                    require_idx: i,
                }),

                // require(<expr>), dynamic, (Also catches require(("x")).)
                _ => out.dynamic.push(tok.start),
            },

            // require "path", parenless call sugar
            Some(TokKind::Str {
                inner_start,
                inner_end,
            }) => {
                let s = toks[i + 1];
                out.sites.push(RequireSite {
                    inner_start,
                    inner_end,
                    tok_start: s.start,
                    tok_end: s.end,
                    has_parens: false,
                    at: tok.start,
                    require_idx: i,
                });
            }

            Some(TokKind::InterpStr) => out.dynamic.push(tok.start),
            // bare require reference, nothing to rewrite but worth counting
            _ => out.dynamic.push(tok.start),
        }
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::syntax::lexer::lex;

    fn spec(src: &str) -> Vec<String> {
        let toks = lex(src).unwrap().toks;
        scan(src, &toks)
            .sites
            .iter()
            .map(|s| src[s.inner_start as usize..s.inner_end as usize].to_string())
            .collect()
    }

    #[test]
    fn finds_plain_requires() {
        assert_eq!(spec(r#"local a = require("./foo")"#), vec!["./foo"]);
        assert_eq!(spec(r#"local a = require('@pkg/x')"#), vec!["@pkg/x"]);
        assert_eq!(spec(r#"local a = require "@pkg/x""#), vec!["@pkg/x"]);
        assert_eq!(spec("local a = require [[@pkg/x]]"), vec!["@pkg/x"]);
    }

    #[test]
    fn skips_members_comments_dynamic() {
        assert!(spec(r#"foo.require("./x")"#).is_empty());
        assert!(spec(r#"foo:require("./x")"#).is_empty());
        assert!(spec(r#"-- require("./x")"#).is_empty());
        assert!(spec(r#"local s = "require('./x')""#).is_empty());
        assert!(spec(r#"require(path)"#).is_empty());
    }

    #[test]
    fn counts_dynamic() {
        let src = r#"require(p) require(`@x/{y}`) local r = require"#;
        let toks = lex(src).unwrap().toks;

        assert_eq!(scan(src, &toks).dynamic.len(), 3);
    }

    #[test]
    fn multiple_sites() {
        let src = r#"
            local A = require("./a") -- comment
            local B = require("../b")
        "#;
        assert_eq!(spec(src), vec!["./a", "../b"]);
    }
}
