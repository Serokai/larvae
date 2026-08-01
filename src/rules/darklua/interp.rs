/*!
remove_interpolated_string

The lexer keeps a backtick string as one opaque token, so the pieces have to
be split out here. The result is a string.format call, `string` strategy
wraps every value in tostring and formats with `%s`, `tostring` strategy
leans on Luau's `%*` instead

Anything the split cannot account for is left alone, that covers a raw
newline in the literal, which no quoted string could hold without moving
line numbers, and a nested backtick string, which this single pass would
never get back to
*/

use crate::rules::engine::{Edit, Flow, RuleCtx, Visit, walk_chunk};
use crate::syntax::ast::*;

/// One piece of a split backtick string
enum Piece {
    /// Literal text, already escaped for a quoted string
    Text(String),
    /// Source text of an interpolated expression
    Value(String),
}

pub fn remove_interpolated_string(ctx: &RuleCtx, edits: &mut Vec<Edit>, strategy: &str) {
    struct V<'a, 'b> {
        ctx: &'a RuleCtx<'b>,
        edits: &'a mut Vec<Edit>,
        tostring_strategy: bool,
    }

    impl Visit for V<'_, '_> {
        fn expr(&mut self, e: &Expr) -> Flow {
            let Expr::InterpString(span) = e else {
                return Flow::Next;
            };
            let raw = self.ctx.text(*span);

            let Some(pieces) = split(raw, self.ctx.quote) else {
                return Flow::Next;
            };

            let text = render(&pieces, self.ctx.quote, self.tostring_strategy);
            let (a, b) = self.ctx.bytes(*span);

            self.edits.push((a, b, text));

            Flow::Next
        }
    }

    walk_chunk(
        ctx.chunk,
        &mut V {
            ctx,
            edits,
            tostring_strategy: strategy == "tostring",
        },
    );
}

/*
Split `\`a {x} b\`` into literal and value pieces

Literal bytes come out ready to sit inside a quoted string, the escape that
Luau only allows in a backtick string is unwrapped and the quote character
we are about to use is escaped. Returns None whenever the transform would
not be safe
*/
fn split(raw: &str, quote: char) -> Option<Vec<Piece>> {
    let body = raw.strip_prefix('`')?.strip_suffix('`')?;
    let bytes = body.as_bytes();
    let mut pieces = Vec::new();
    let mut text = String::new();
    let mut i = 0usize;

    while i < bytes.len() {
        match bytes[i] {
            b'\\' => {
                let next = *bytes.get(i + 1)?;

                match next {
                    // only meaningful inside backticks, a quoted string takes it plain
                    b'{' => text.push('{'),

                    b'`' => text.push('`'),

                    _ => {
                        let c = body[i + 1..].chars().next()?;
                        text.push('\\');
                        text.push(c);
                        i += 1 + c.len_utf8();
                        continue;
                    }
                }

                i += 2;
            }

            b'{' => {
                let (expr, next) = scan_expr(body, i)?;
                // a nested backtick string would never be visited again
                if expr.contains('`') || expr.trim().is_empty() {
                    return None;
                }

                pieces.push(Piece::Text(std::mem::take(&mut text)));
                pieces.push(Piece::Value(expr.to_string()));
                i = next;
            }

            // a quoted string cannot hold a raw newline without shifting lines
            b'\n' => return None,

            b'%' => {
                text.push_str("%%");
                i += 1;
            }

            c if c == quote as u8 => {
                text.push('\\');
                text.push(quote);
                i += 1;
            }

            _ => {
                let c = body[i..].chars().next()?;
                text.push(c);
                i += c.len_utf8();
            }
        }
    }

    pieces.push(Piece::Text(text));

    Some(pieces)
}

/// Find the expression inside `{...}`, returns its text and the offset past `}`
fn scan_expr(body: &str, open: usize) -> Option<(&str, usize)> {
    let bytes = body.as_bytes();
    let mut depth = 0usize;
    let mut i = open;

    while i < bytes.len() {
        match bytes[i] {
            b'{' => {
                depth += 1;
                i += 1;
            }

            b'}' => {
                depth -= 1;
                i += 1;

                if depth == 0 {
                    return Some((&body[open + 1..i - 1], i));
                }
            }

            // a string inside the braces must not unbalance the scan
            q @ (b'"' | b'\'') => {
                i += 1;

                while i < bytes.len() && bytes[i] != q {
                    if bytes[i] == b'\\' {
                        i += 1;
                    }

                    i += 1;
                }

                i += 1;
            }

            b'\\' => i += 2,

            _ => i += 1,
        }
    }

    None
}

fn render(pieces: &[Piece], quote: char, tostring_strategy: bool) -> String {
    let values: Vec<&String> = pieces
        .iter()
        .filter_map(|p| match p {
            Piece::Value(v) => Some(v),

            Piece::Text(_) => None,
        })
        .collect();

    let mut format = String::new();

    for p in pieces {
        match p {
            Piece::Text(t) => format.push_str(t),

            Piece::Value(_) => format.push_str(if tostring_strategy { "%*" } else { "%s" }),
        }
    }

    // nothing to interpolate, it was only ever a string
    if values.is_empty() {
        // the doubled percents were for a format string we are not emitting
        return format!("{quote}{}{quote}", format.replace("%%", "%"));
    }

    let args: Vec<String> = values
        .iter()
        .map(|v| {
            if tostring_strategy {
                v.trim().to_string()
            } else {
                format!("tostring({})", v.trim())
            }
        })
        .collect();
    format!("string.format({quote}{format}{quote}, {})", args.join(", "))
}

#[cfg(test)]
mod tests {
    use super::super::testing::run;
    use super::*;

    fn as_string(ctx: &RuleCtx, edits: &mut Vec<Edit>) {
        remove_interpolated_string(ctx, edits, "string");
    }

    fn as_tostring(ctx: &RuleCtx, edits: &mut Vec<Edit>) {
        remove_interpolated_string(ctx, edits, "tostring");
    }

    #[test]
    fn string_strategy_wraps_each_value() {
        assert_eq!(
            run("local s = `hello {name}`\n", as_string),
            "local s = string.format(\"hello %s\", tostring(name))\n"
        );
        assert_eq!(
            run("local s = `{a} and {b}`\n", as_string),
            "local s = string.format(\"%s and %s\", tostring(a), tostring(b))\n"
        );
    }

    #[test]
    fn tostring_strategy_uses_the_star_format() {
        assert_eq!(
            run("local s = `hello {name}`\n", as_tostring),
            "local s = string.format(\"hello %*\", name)\n"
        );
    }

    #[test]
    fn percent_signs_are_escaped() {
        assert_eq!(
            run("local s = `100% of {n}`\n", as_string),
            "local s = string.format(\"100%% of %s\", tostring(n))\n"
        );
    }

    #[test]
    fn a_plain_backtick_string_becomes_a_plain_string() {
        assert_eq!(
            run("local s = `hello`\n", as_string),
            "local s = \"hello\"\n"
        );
        // the percent needs no doubling when there is no format call
        assert_eq!(run("local s = `50%`\n", as_string), "local s = \"50%\"\n");
    }

    #[test]
    fn escapes_are_carried_across() {
        assert_eq!(
            run("local s = `a\\nb {x}`\n", as_string),
            "local s = string.format(\"a\\nb %s\", tostring(x))\n"
        );
        // an escaped brace is just a brace once the backticks are gone
        assert_eq!(run("local s = `a\\{b`\n", as_string), "local s = \"a{b\"\n");
    }

    #[test]
    fn quotes_inside_get_escaped() {
        assert_eq!(
            run("local s = `say \"hi\" to {n}`\n", as_string),
            "local s = string.format(\"say \\\"hi\\\" to %s\", tostring(n))\n"
        );
    }

    #[test]
    fn expressions_keep_their_source() {
        assert_eq!(
            run("local s = `{a + b}`\n", as_string),
            "local s = string.format(\"%s\", tostring(a + b))\n"
        );
        assert_eq!(
            run("local s = `{t[\"k\"]}`\n", as_string),
            "local s = string.format(\"%s\", tostring(t[\"k\"]))\n"
        );
    }

    #[test]
    fn nested_backticks_are_left_alone() {
        // one pass could never reach the inner string again
        let src = "local s = `outer {`inner {x}`}`\n";
        assert_eq!(run(src, as_string), src);
    }

    #[test]
    fn ordinary_strings_are_untouched() {
        let src = "local s = \"hello\"\nlocal t = 'x'\n";
        assert_eq!(run(src, as_string), src);
    }
}
