/*!
A very small constant evaluator

It only has to answer one question, does this expression have a value we can
write back into the source without changing what the program does. So it
folds nothing it cannot print exactly, numbers have to land on a whole f64
inside the range doubles represent exactly, and strings have to be literals
with no escapes so the bytes going out are the bytes that came in
*/

use super::support;
use crate::rules::engine::RuleCtx;
use crate::syntax::ast::*;

/// The largest whole number an f64 still represents exactly
const SAFE_INT: f64 = 9_007_199_254_740_992.0;

#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Nil,
    Bool(bool),
    Num(f64),
    Str(String),
}

impl Value {
    /// Lua truthiness, only nil and false are false, zero is not
    pub fn truthy(&self) -> bool {
        !matches!(self, Value::Nil | Value::Bool(false))
    }
}

/// Value of an expression when it is a compile time constant
pub fn eval(ctx: &RuleCtx, e: &Expr) -> Option<Value> {
    match e {
        Expr::Nil(_) => Some(Value::Nil),

        Expr::True(_) => Some(Value::Bool(true)),

        Expr::False(_) => Some(Value::Bool(false)),

        Expr::Number(span) => parse_number(ctx.text(*span)).map(Value::Num),

        Expr::String(span) => {
            support::plain_string_value(ctx, *span).map(|s| Value::Str(s.to_string()))
        }

        Expr::Paren { inner, .. } => eval(ctx, inner),

        Expr::Unary { op, operand, .. } => {
            let v = eval(ctx, operand)?;

            match ctx.text(*op) {
                "-" => match v {
                    Value::Num(n) => Some(Value::Num(-n)),

                    _ => None,
                },

                "not" => Some(Value::Bool(!v.truthy())),
                // `#` would need to know the length of a real value
                _ => None,
            }
        }

        Expr::Binary { op, lhs, rhs, .. } => {
            let name = ctx.text(*op);
            // the short circuit operators only need the left side to decide
            if name == "and" || name == "or" {
                let l = eval(ctx, lhs)?;
                let r = eval(ctx, rhs)?;

                let take_left = if name == "and" {
                    !l.truthy()
                } else {
                    l.truthy()
                };

                return Some(if take_left { l } else { r });
            }

            binary(name, eval(ctx, lhs)?, eval(ctx, rhs)?)
        }

        _ => None,
    }
}

fn binary(op: &str, l: Value, r: Value) -> Option<Value> {
    use Value::*;

    match op {
        "+" | "-" | "*" | "/" | "%" | "^" => {
            let (Num(a), Num(b)) = (l, r) else {
                return None;
            };

            Some(Num(match op {
                "+" => a + b,

                "-" => a - b,

                "*" => a * b,

                "/" => a / b,
                // Lua's modulo follows the divisor's sign, Rust's does not
                "%" => a - (a / b).floor() * b,

                _ => a.powf(b),
            }))
        }

        ".." => {
            let (Str(a), Str(b)) = (l, r) else {
                return None;
            };

            Some(Str(format!("{a}{b}")))
        }

        "==" => Some(Bool(equal(&l, &r))),

        "~=" => Some(Bool(!equal(&l, &r))),

        "<" | "<=" | ">" | ">=" => {
            let ord = match (&l, &r) {
                (Num(a), Num(b)) => a.partial_cmp(b)?,

                (Str(a), Str(b)) => a.cmp(b),

                _ => return None,
            };

            Some(Bool(match op {
                "<" => ord.is_lt(),

                "<=" => ord.is_le(),

                ">" => ord.is_gt(),

                _ => ord.is_ge(),
            }))
        }

        _ => None,
    }
}

/// Lua equality never coerces across types
fn equal(l: &Value, r: &Value) -> bool {
    match (l, r) {
        (Value::Num(a), Value::Num(b)) => a == b,

        (Value::Str(a), Value::Str(b)) => a == b,

        (Value::Bool(a), Value::Bool(b)) => a == b,

        (Value::Nil, Value::Nil) => true,

        _ => false,
    }
}

/// Every numeric literal Luau writes, separators included
pub fn parse_number(text: &str) -> Option<f64> {
    let cleaned: String = text.chars().filter(|&c| c != '_').collect();
    let lower = cleaned.to_ascii_lowercase();

    if let Some(hex) = lower.strip_prefix("0x") {
        // a hex float carries a binary exponent, not worth the rounding risk
        if hex.is_empty() || hex.contains('p') || hex.contains('.') {
            return None;
        }

        return u64::from_str_radix(hex, 16).ok().map(|v| v as f64);
    }

    if let Some(bits) = lower.strip_prefix("0b") {
        if bits.is_empty() || !bits.chars().all(|c| c == '0' || c == '1') {
            return None;
        }

        return u64::from_str_radix(bits, 2).ok().map(|v| v as f64);
    }

    cleaned.parse::<f64>().ok()
}

/*
Print a value back as Luau source, None when the result would not round
trip, that is the whole safety story for compute_expression
*/
pub fn print(v: &Value, quote: char) -> Option<String> {
    match v {
        Value::Nil => Some("nil".to_string()),

        Value::Bool(true) => Some("true".to_string()),

        Value::Bool(false) => Some("false".to_string()),

        Value::Num(n) => {
            // fractions would need a rounding policy, whole numbers do not
            if !n.is_finite() || n.fract() != 0.0 || n.abs() > SAFE_INT {
                return None;
            }

            Some(format!("{}", *n as i64))
        }

        Value::Str(s) => {
            if s.contains(quote) || s.contains('\\') || s.contains('\n') || s.contains('\r') {
                return None;
            }

            Some(format!("{quote}{s}{quote}"))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::syntax::{lexer, parser};

    fn value(src: &str) -> Option<Value> {
        let full = format!("local _ = {src}");
        let lexed = lexer::lex(&full).unwrap();
        let chunk = parser::parse(&full, &lexed.toks).unwrap();

        let ctx = RuleCtx {
            src: &full,
            toks: &lexed.toks,
            chunk: &chunk,
            comments: &lexed.comments,
            require_forms: &[],
            dm_path: None,
            quote: '"',
        };

        let Stmt::Local(l) = &chunk.block.stmts[0] else {
            panic!()
        };

        eval(&ctx, &l.values[0])
    }

    fn shown(src: &str) -> Option<String> {
        print(&value(src)?, '"')
    }

    #[test]
    fn arithmetic_folds_when_it_stays_whole() {
        assert_eq!(shown("1 + 2").as_deref(), Some("3"));
        assert_eq!(shown("10 / 2").as_deref(), Some("5"));
        assert_eq!(shown("2 ^ 10").as_deref(), Some("1024"));
        assert_eq!(shown("7 % 3").as_deref(), Some("1"));
        assert_eq!(shown("2 * 3 + 4").as_deref(), Some("10"));
        assert_eq!(shown("-(3 - 5)").as_deref(), Some("2"));
    }

    #[test]
    fn fractions_and_nonsense_do_not_fold() {
        // a fraction would need a rounding policy we do not want to guess
        assert_eq!(shown("10 / 4"), None);
        assert_eq!(shown("1 / 0"), None);
        assert_eq!(shown("0 / 0"), None);
        // beyond exact doubles the printed form would lie
        assert_eq!(shown("9007199254740992 * 2"), None);
    }

    #[test]
    fn lua_modulo_follows_the_divisor() {
        assert_eq!(shown("-1 % 3").as_deref(), Some("2"));
    }

    #[test]
    fn comparisons_and_logic_fold() {
        assert_eq!(shown("1 < 2").as_deref(), Some("true"));
        assert_eq!(shown("\"a\" == \"a\"").as_deref(), Some("true"));
        assert_eq!(shown("1 == \"1\"").as_deref(), Some("false"));
        assert_eq!(shown("not nil").as_deref(), Some("true"));
        // zero is truthy in Lua
        assert_eq!(shown("not 0").as_deref(), Some("false"));
        assert_eq!(shown("true and 2").as_deref(), Some("2"));
        assert_eq!(shown("false and 2").as_deref(), Some("false"));
        assert_eq!(shown("nil or 3").as_deref(), Some("3"));
    }

    #[test]
    fn string_concat_folds_only_for_plain_literals() {
        assert_eq!(shown("\"a\" .. \"b\"").as_deref(), Some("\"ab\""));
        // an escape means the bytes are not what they look like
        assert_eq!(shown("\"a\\n\" .. \"b\""), None);
        assert_eq!(shown("1 .. 2"), None);
    }

    #[test]
    fn anything_with_a_name_in_it_is_not_constant() {
        assert_eq!(value("x + 1"), None);
        assert_eq!(value("f()"), None);
        assert_eq!(value("#t"), None);
    }

    #[test]
    fn number_literals_parse_the_way_luau_writes_them() {
        assert_eq!(parse_number("0b1010"), Some(10.0));
        assert_eq!(parse_number("0xFF"), Some(255.0));
        assert_eq!(parse_number("1_000"), Some(1000.0));
        assert_eq!(parse_number("1.5e3"), Some(1500.0));
    }
}
