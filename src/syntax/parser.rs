/*!
Recursive descent parser for the full Luau grammar, modeled on the official
Parser.cpp, it produces the token span tree in [`crate::syntax::ast`]

Types are parsed for their extent but not interpreted, that is deliberate,
a rule that needs type structure can parse the span later, recursion is
depth guarded so pathological nesting is a clean error and never a crash
*/

use crate::syntax::ast::*;
use crate::syntax::lexer::{Tok, TokKind};

#[derive(Debug)]
pub struct ParseError {
    pub offset: usize,
    pub message: String,
}

/// Nesting limit, deep enough for real code and shallow enough to not blow
/// the stack, darklua's crash class does not exist here
const MAX_DEPTH: u32 = 180;
const UNARY_PRIORITY: u8 = 12;

pub fn parse(src: &str, toks: &[Tok]) -> Result<Chunk, ParseError> {
    let mut p = Parser {
        src,
        toks,
        pos: 0,
        depth: 0,
    };
    let block = p.block()?;
    if !p.at_end() {
        return Err(p.err("unexpected token"));
    }
    Ok(Chunk { block })
}

struct Parser<'a> {
    src: &'a str,
    toks: &'a [Tok],
    pos: usize,
    depth: u32,
}

impl<'a> Parser<'a> {
    // --- token access ------------------------------------------------------

    fn at_end(&self) -> bool {
        self.pos >= self.toks.len()
    }

    fn text_at(&self, n: usize) -> &'a str {
        match self.toks.get(self.pos + n) {
            Some(t) => t.text(self.src),
            None => "",
        }
    }

    fn text(&self) -> &'a str {
        self.text_at(0)
    }

    fn kind_at(&self, n: usize) -> Option<TokKind> {
        self.toks.get(self.pos + n).map(|t| t.kind)
    }

    fn at(&self, s: &str) -> bool {
        self.text() == s
    }

    fn at_name(&self) -> bool {
        matches!(self.kind_at(0), Some(TokKind::Ident)) && !is_reserved(self.text())
    }

    fn bump(&mut self) -> usize {
        let i = self.pos;
        self.pos += 1;
        i
    }

    fn eat(&mut self, s: &str) -> bool {
        if self.at(s) {
            self.bump();
            true
        } else {
            false
        }
    }

    fn expect(&mut self, s: &str) -> Result<usize, ParseError> {
        if self.at(s) {
            Ok(self.bump())
        } else {
            Err(self.err(&format!("expected `{s}`, found {}", self.found())))
        }
    }

    fn expect_name(&mut self) -> Result<TokSpan, ParseError> {
        if self.at_name() {
            let i = self.bump();
            Ok(TokSpan::new(i, i + 1))
        } else {
            Err(self.err(&format!("expected a name, found {}", self.found())))
        }
    }

    fn found(&self) -> String {
        if self.at_end() {
            "end of file".to_string()
        } else {
            format!("`{}`", self.text())
        }
    }

    fn err(&self, message: &str) -> ParseError {
        let offset = match self.toks.get(self.pos) {
            Some(t) => t.start as usize,
            None => self.src.len(),
        };
        ParseError {
            offset,
            message: message.to_string(),
        }
    }

    fn enter(&mut self) -> Result<(), ParseError> {
        self.depth += 1;
        if self.depth > MAX_DEPTH {
            return Err(self.err("expression or statement nests too deeply"));
        }
        Ok(())
    }

    fn leave(&mut self) {
        self.depth -= 1;
    }

    // --- statements --------------------------------------------------------

    fn at_block_end(&self) -> bool {
        matches!(self.text(), "end" | "else" | "elseif" | "until")
    }

    fn block(&mut self) -> Result<Block, ParseError> {
        self.enter()?;
        let start = self.pos;
        let mut stmts = Vec::new();
        while !self.at_end() && !self.at_block_end() {
            let is_return = self.at("return");
            stmts.push(self.stmt()?);
            if is_return {
                // a return ends its block, only `;` may follow
                if self.at(";") {
                    let i = self.bump();
                    stmts.push(Stmt::Empty(TokSpan::new(i, i + 1)));
                }
                break;
            }
        }
        self.leave();
        Ok(Block {
            stmts,
            span: TokSpan::new(start, self.pos),
        })
    }

    fn stmt(&mut self) -> Result<Stmt, ParseError> {
        self.enter()?;
        let r = self.stmt_inner();
        self.leave();
        r
    }

    fn stmt_inner(&mut self) -> Result<Stmt, ParseError> {
        let start = self.pos;
        match self.text() {
            ";" => {
                self.bump();
                Ok(Stmt::Empty(TokSpan::new(start, self.pos)))
            }
            "if" => self.if_stmt(start),
            "while" => {
                self.bump();
                let cond = self.expr()?;
                self.expect("do")?;
                let block = self.block()?;
                self.expect("end")?;
                Ok(Stmt::While(While {
                    cond,
                    block,
                    span: TokSpan::new(start, self.pos),
                }))
            }
            "do" => {
                self.bump();
                let block = self.block()?;
                self.expect("end")?;
                Ok(Stmt::Do(DoBlock {
                    block,
                    span: TokSpan::new(start, self.pos),
                }))
            }
            "for" => self.for_stmt(start),
            "repeat" => {
                self.bump();
                let block = self.block()?;
                self.expect("until")?;
                let cond = self.expr()?;
                Ok(Stmt::Repeat(Repeat {
                    block,
                    cond,
                    span: TokSpan::new(start, self.pos),
                }))
            }
            "function" => self.function_stmt(start, Vec::new()),
            "local" | "const" => self.local_stmt(start),
            "return" => {
                self.bump();
                let values = if self.at_end() || self.at_block_end() || self.at(";") {
                    Vec::new()
                } else {
                    self.expr_list()?
                };
                Ok(Stmt::Return(Return {
                    values,
                    span: TokSpan::new(start, self.pos),
                }))
            }
            "break" => {
                self.bump();
                Ok(Stmt::Break(TokSpan::new(start, self.pos)))
            }
            "continue" if self.continue_is_keyword() => {
                self.bump();
                Ok(Stmt::Continue(TokSpan::new(start, self.pos)))
            }
            "@" => {
                let attributes = self.attributes()?;
                if self.at("local") {
                    self.bump();
                    self.local_function(start, attributes)
                } else {
                    self.function_stmt(start, attributes)
                }
            }
            "export" if self.text_at(1) == "type" => self.type_alias(start),
            "type" if self.type_is_alias() => self.type_alias(start),
            _ => self.expr_stmt(start),
        }
    }

    /// `continue` is contextual, it is only the keyword when nothing that
    /// would continue an expression follows
    fn continue_is_keyword(&self) -> bool {
        !matches!(
            self.text_at(1),
            "=" | "," | "." | "(" | "[" | ":" | "+=" | "-=" | "*=" | "/=" | "%=" | "^=" | "..="
        )
    }

    /// `type` is contextual too, `type X =`, `type X<`, `type function f`
    fn type_is_alias(&self) -> bool {
        if self.text_at(1) == "function" {
            return true;
        }
        matches!(self.kind_at(1), Some(TokKind::Ident))
            && !is_reserved(self.text_at(1))
            && matches!(self.text_at(2), "=" | "<")
    }

    fn attributes(&mut self) -> Result<Vec<TokSpan>, ParseError> {
        let mut out = Vec::new();
        while self.at("@") {
            let start = self.bump();
            self.expect_name()?;
            out.push(TokSpan::new(start, self.pos));
        }
        Ok(out)
    }

    fn if_stmt(&mut self, start: usize) -> Result<Stmt, ParseError> {
        self.expect("if")?;
        let mut branches = Vec::new();
        let cond = self.expr()?;
        self.expect("then")?;
        branches.push((cond, self.block()?));
        while self.at("elseif") {
            self.bump();
            let cond = self.expr()?;
            self.expect("then")?;
            branches.push((cond, self.block()?));
        }
        let else_block = if self.eat("else") {
            Some(self.block()?)
        } else {
            None
        };
        self.expect("end")?;
        Ok(Stmt::If(If {
            branches,
            else_block,
            span: TokSpan::new(start, self.pos),
        }))
    }

    fn for_stmt(&mut self, start: usize) -> Result<Stmt, ParseError> {
        self.expect("for")?;
        let first = self.binding()?;
        if self.eat("=") {
            let from = self.expr()?;
            self.expect(",")?;
            let limit = self.expr()?;
            let step = if self.eat(",") {
                Some(self.expr()?)
            } else {
                None
            };
            self.expect("do")?;
            let block = self.block()?;
            self.expect("end")?;
            return Ok(Stmt::NumericFor(NumericFor {
                var: first,
                start: from,
                limit,
                step,
                block,
                span: TokSpan::new(start, self.pos),
            }));
        }
        let mut vars = vec![first];
        while self.eat(",") {
            vars.push(self.binding()?);
        }
        self.expect("in")?;
        let exprs = self.expr_list()?;
        self.expect("do")?;
        let block = self.block()?;
        self.expect("end")?;
        Ok(Stmt::GenericFor(GenericFor {
            vars,
            exprs,
            block,
            span: TokSpan::new(start, self.pos),
        }))
    }

    fn local_stmt(&mut self, start: usize) -> Result<Stmt, ParseError> {
        let is_const = self.at("const");
        self.bump();
        if self.at("function") {
            return self.local_function(start, Vec::new());
        }
        if self.at("@") {
            let attributes = self.attributes()?;
            return self.local_function(start, attributes);
        }
        let keyword = TokSpan::new(start, start + 1);
        let mut names = vec![self.binding()?];
        while self.eat(",") {
            names.push(self.binding()?);
        }
        let values = if self.eat("=") {
            self.expr_list()?
        } else {
            Vec::new()
        };
        Ok(Stmt::Local(Local {
            keyword,
            is_const,
            names,
            values,
            span: TokSpan::new(start, self.pos),
        }))
    }

    fn local_function(
        &mut self,
        start: usize,
        attributes: Vec<TokSpan>,
    ) -> Result<Stmt, ParseError> {
        self.expect("function")?;
        let name = self.expect_name()?;
        let body = self.function_body()?;
        Ok(Stmt::LocalFunction(LocalFunction {
            attributes,
            name,
            body,
            span: TokSpan::new(start, self.pos),
        }))
    }

    fn function_stmt(
        &mut self,
        start: usize,
        attributes: Vec<TokSpan>,
    ) -> Result<Stmt, ParseError> {
        self.expect("function")?;
        let mut path = vec![self.expect_name()?];
        let mut is_method = false;
        loop {
            if self.eat(".") {
                path.push(self.expect_name()?);
            } else if self.at(":") {
                self.bump();
                path.push(self.expect_name()?);
                is_method = true;
                break;
            } else {
                break;
            }
        }
        let body = self.function_body()?;
        Ok(Stmt::Function(Function {
            attributes,
            path,
            is_method,
            body,
            span: TokSpan::new(start, self.pos),
        }))
    }

    fn function_body(&mut self) -> Result<FunctionBody, ParseError> {
        let start = self.pos;
        let generics = if self.at("<") {
            Some(self.angle_span()?)
        } else {
            None
        };
        self.expect("(")?;
        let mut params = Vec::new();
        if !self.at(")") {
            loop {
                if self.at("...") {
                    let i = self.bump();
                    let ty = if self.eat(":") {
                        Some(self.type_()?)
                    } else {
                        None
                    };
                    params.push(Param {
                        name: TokSpan::new(i, i + 1),
                        is_vararg: true,
                        ty,
                    });
                    break;
                }
                let b = self.binding()?;
                params.push(Param {
                    name: b.name,
                    is_vararg: false,
                    ty: b.ty,
                });
                if !self.eat(",") {
                    break;
                }
            }
        }
        self.expect(")")?;
        let ret_type = if self.eat(":") {
            Some(self.type_ret()?)
        } else {
            None
        };
        let block = self.block()?;
        self.expect("end")?;
        Ok(FunctionBody {
            generics,
            params,
            ret_type,
            block,
            span: TokSpan::new(start, self.pos),
        })
    }

    fn binding(&mut self) -> Result<Binding, ParseError> {
        let name = self.expect_name()?;
        let ty = if self.eat(":") {
            Some(self.type_()?)
        } else {
            None
        };
        Ok(Binding { name, ty })
    }

    fn type_alias(&mut self, start: usize) -> Result<Stmt, ParseError> {
        let exported = self.eat("export");
        self.expect("type")?;
        if self.at("function") {
            // `type function f() ... end`, a user defined type function
            self.bump();
            let name = self.expect_name()?;
            self.function_body()?;
            return Ok(Stmt::TypeAlias(TypeAlias {
                exported,
                name,
                span: TokSpan::new(start, self.pos),
            }));
        }
        let name = self.expect_name()?;
        if self.at("<") {
            self.angle_span()?;
        }
        self.expect("=")?;
        self.type_()?;
        Ok(Stmt::TypeAlias(TypeAlias {
            exported,
            name,
            span: TokSpan::new(start, self.pos),
        }))
    }

    fn expr_stmt(&mut self, start: usize) -> Result<Stmt, ParseError> {
        let first = self.suffixed_expr()?;
        // assignment, plain or compound
        if self.at("=") || self.at(",") || is_compound_op(self.text()) {
            let mut targets = vec![first];
            while self.eat(",") {
                targets.push(self.suffixed_expr()?);
            }
            let op_idx = if is_compound_op(self.text()) {
                self.bump()
            } else {
                self.expect("=")?
            };
            let values = self.expr_list()?;
            return Ok(Stmt::Assign(Assign {
                targets,
                op: TokSpan::new(op_idx, op_idx + 1),
                values,
                span: TokSpan::new(start, self.pos),
            }));
        }
        match &first {
            Expr::Call { .. } => Ok(Stmt::Call(first, TokSpan::new(start, self.pos))),
            _ => Err(self.err("this expression is not a statement")),
        }
    }

    fn expr_list(&mut self) -> Result<Vec<Expr>, ParseError> {
        let mut out = vec![self.expr()?];
        while self.eat(",") {
            out.push(self.expr()?);
        }
        Ok(out)
    }

    // --- expressions -------------------------------------------------------

    fn expr(&mut self) -> Result<Expr, ParseError> {
        self.sub_expr(0)
    }

    fn sub_expr(&mut self, limit: u8) -> Result<Expr, ParseError> {
        self.enter()?;
        let start = self.pos;
        let mut left = if is_unary_op(self.text()) {
            let op = self.bump();
            let operand = self.sub_expr(UNARY_PRIORITY)?;
            Expr::Unary {
                op: TokSpan::new(op, op + 1),
                operand: Box::new(operand),
                span: TokSpan::new(start, self.pos),
            }
        } else {
            self.simple_expr()?
        };

        while let Some((left_prec, right_prec)) = binop_priority(self.text()) {
            if left_prec <= limit {
                break;
            }
            let op = self.bump();
            let rhs = self.sub_expr(right_prec)?;
            left = Expr::Binary {
                op: TokSpan::new(op, op + 1),
                lhs: Box::new(left),
                rhs: Box::new(rhs),
                span: TokSpan::new(start, self.pos),
            };
        }
        self.leave();
        Ok(left)
    }

    fn simple_expr(&mut self) -> Result<Expr, ParseError> {
        let start = self.pos;
        let mut e = match self.text() {
            "nil" => {
                self.bump();
                Expr::Nil(TokSpan::new(start, self.pos))
            }
            "true" => {
                self.bump();
                Expr::True(TokSpan::new(start, self.pos))
            }
            "false" => {
                self.bump();
                Expr::False(TokSpan::new(start, self.pos))
            }
            "..." => {
                self.bump();
                Expr::Vararg(TokSpan::new(start, self.pos))
            }
            "function" => {
                self.bump();
                let body = self.function_body()?;
                Expr::Function {
                    attributes: Vec::new(),
                    body: Box::new(body),
                    span: TokSpan::new(start, self.pos),
                }
            }
            "@" => {
                let attributes = self.attributes()?;
                self.expect("function")?;
                let body = self.function_body()?;
                Expr::Function {
                    attributes,
                    body: Box::new(body),
                    span: TokSpan::new(start, self.pos),
                }
            }
            "{" => self.table_expr()?,
            "if" => self.if_else_expr()?,
            _ => match self.kind_at(0) {
                Some(TokKind::Number) => {
                    self.bump();
                    Expr::Number(TokSpan::new(start, self.pos))
                }
                Some(TokKind::Str { .. }) => {
                    self.bump();
                    Expr::String(TokSpan::new(start, self.pos))
                }
                Some(TokKind::InterpStr) => {
                    self.bump();
                    Expr::InterpString(TokSpan::new(start, self.pos))
                }
                _ => self.suffixed_expr()?,
            },
        };
        // `expr :: T` binds tighter than any binary operator
        while self.at("::") {
            self.bump();
            let ty = self.type_()?;
            e = Expr::TypeAssert {
                expr: Box::new(e),
                ty,
                span: TokSpan::new(start, self.pos),
            };
        }
        Ok(e)
    }

    fn primary_expr(&mut self) -> Result<Expr, ParseError> {
        let start = self.pos;
        if self.at("(") {
            self.bump();
            let inner = self.expr()?;
            self.expect(")")?;
            return Ok(Expr::Paren {
                inner: Box::new(inner),
                span: TokSpan::new(start, self.pos),
            });
        }
        let name = self.expect_name()?;
        Ok(Expr::Name(name))
    }

    fn suffixed_expr(&mut self) -> Result<Expr, ParseError> {
        self.enter()?;
        let start = self.pos;
        let mut e = self.primary_expr()?;
        loop {
            match self.text() {
                "." => {
                    self.bump();
                    let field = self.expect_name()?;
                    e = Expr::Index {
                        object: Box::new(e),
                        key: IndexKey::Field(field),
                        span: TokSpan::new(start, self.pos),
                    };
                }
                "[" => {
                    self.bump();
                    let key = self.expr()?;
                    self.expect("]")?;
                    e = Expr::Index {
                        object: Box::new(e),
                        key: IndexKey::Computed(Box::new(key)),
                        span: TokSpan::new(start, self.pos),
                    };
                }
                ":" => {
                    self.bump();
                    let method = self.expect_name()?;
                    let args = self.call_args()?;
                    e = Expr::Call {
                        func: Box::new(e),
                        method: Some(method),
                        args,
                        span: TokSpan::new(start, self.pos),
                    };
                }
                "(" | "{" => {
                    let args = self.call_args()?;
                    e = Expr::Call {
                        func: Box::new(e),
                        method: None,
                        args,
                        span: TokSpan::new(start, self.pos),
                    };
                }
                _ => {
                    if matches!(self.kind_at(0), Some(TokKind::Str { .. })) {
                        let args = self.call_args()?;
                        e = Expr::Call {
                            func: Box::new(e),
                            method: None,
                            args,
                            span: TokSpan::new(start, self.pos),
                        };
                    } else {
                        break;
                    }
                }
            }
        }
        self.leave();
        Ok(e)
    }

    fn call_args(&mut self) -> Result<CallArgs, ParseError> {
        if self.at("(") {
            self.bump();
            let args = if self.at(")") {
                Vec::new()
            } else {
                self.expr_list()?
            };
            self.expect(")")?;
            return Ok(CallArgs::Paren(args));
        }
        if self.at("{") {
            let table = self.table_expr()?;
            return Ok(CallArgs::Table(Box::new(table)));
        }
        if matches!(self.kind_at(0), Some(TokKind::Str { .. })) {
            let i = self.bump();
            return Ok(CallArgs::Str(TokSpan::new(i, i + 1)));
        }
        Err(self.err(&format!("expected call arguments, found {}", self.found())))
    }

    fn table_expr(&mut self) -> Result<Expr, ParseError> {
        let start = self.pos;
        self.expect("{")?;
        let mut fields = Vec::new();
        while !self.at("}") {
            if self.at_end() {
                return Err(self.err("unterminated table"));
            }
            if self.at("[") {
                self.bump();
                let key = self.expr()?;
                self.expect("]")?;
                self.expect("=")?;
                let value = self.expr()?;
                fields.push(TableField::Computed { key, value });
            } else if self.at_name() && self.text_at(1) == "=" {
                let name = self.expect_name()?;
                self.bump();
                let value = self.expr()?;
                fields.push(TableField::Named { name, value });
            } else {
                fields.push(TableField::Positional(self.expr()?));
            }
            if !self.eat(",") && !self.eat(";") {
                break;
            }
        }
        self.expect("}")?;
        Ok(Expr::Table {
            fields,
            span: TokSpan::new(start, self.pos),
        })
    }

    fn if_else_expr(&mut self) -> Result<Expr, ParseError> {
        let start = self.pos;
        self.expect("if")?;
        let mut branches = Vec::new();
        let cond = self.expr()?;
        self.expect("then")?;
        branches.push((cond, self.expr()?));
        while self.at("elseif") {
            self.bump();
            let cond = self.expr()?;
            self.expect("then")?;
            branches.push((cond, self.expr()?));
        }
        self.expect("else")?;
        let else_value = self.expr()?;
        Ok(Expr::IfElse {
            branches,
            else_value: Box::new(else_value),
            span: TokSpan::new(start, self.pos),
        })
    }

    // --- types, extent only ------------------------------------------------

    /// Consume a balanced `<...>` group, used for generic parameter lists
    fn angle_span(&mut self) -> Result<TokSpan, ParseError> {
        let start = self.pos;
        self.expect("<")?;
        let mut depth = 1usize;
        while depth > 0 {
            if self.at_end() {
                return Err(self.err("unterminated generic parameter list"));
            }
            match self.text() {
                "<" => depth += 1,
                ">" => depth -= 1,
                ">=" if depth == 1 => {
                    return Err(self.err("write `> =` here, `>=` reads as one operator"));
                }
                _ => {}
            }
            self.bump();
        }
        Ok(TokSpan::new(start, self.pos))
    }

    fn type_(&mut self) -> Result<TokSpan, ParseError> {
        self.enter()?;
        let start = self.pos;
        let r = self.type_body();
        self.leave();
        r?;
        Ok(TokSpan::new(start, self.pos))
    }

    fn type_body(&mut self) -> Result<(), ParseError> {
        // Luau allows a leading `|` or `&`
        if self.at("|") || self.at("&") {
            self.bump();
        }
        self.type_suffixed()?;
        while self.at("|") || self.at("&") {
            self.bump();
            self.type_suffixed()?;
        }
        Ok(())
    }

    /// A return type, either a single type or a parenthesized pack
    fn type_ret(&mut self) -> Result<TokSpan, ParseError> {
        self.type_()
    }

    fn type_suffixed(&mut self) -> Result<(), ParseError> {
        self.type_primary()?;
        loop {
            if self.at("?") {
                self.bump();
            } else if self.at("->") {
                self.bump();
                self.type_suffixed()?;
            } else {
                break;
            }
        }
        Ok(())
    }

    fn type_primary(&mut self) -> Result<(), ParseError> {
        self.enter()?;
        let r = self.type_primary_inner();
        self.leave();
        r
    }

    fn type_primary_inner(&mut self) -> Result<(), ParseError> {
        match self.text() {
            "nil" | "true" | "false" => {
                self.bump();
                Ok(())
            }
            "typeof" if self.text_at(1) == "(" => {
                self.bump();
                self.bump();
                self.expr()?;
                self.expect(")")?;
                Ok(())
            }
            "..." => {
                // variadic element of a type pack
                self.bump();
                self.type_suffixed()
            }
            // a generic function type, `<T>(T) -> T`
            "<" => {
                self.angle_span()?;
                self.type_primary_inner()
            }
            "(" => {
                // either a parenthesized type or a function type's parameters
                self.bump();
                if !self.at(")") {
                    loop {
                        if self.at("...") {
                            self.bump();
                            if !self.at(")") && !self.at(",") {
                                self.type_suffixed()?;
                            }
                        } else {
                            // an optionally named parameter
                            if self.at_name() && self.text_at(1) == ":" {
                                self.bump();
                                self.bump();
                            }
                            self.type_body()?;
                        }
                        if !self.eat(",") {
                            break;
                        }
                    }
                }
                self.expect(")")?;
                Ok(())
            }
            "{" => self.type_table(),
            _ => match self.kind_at(0) {
                Some(TokKind::Str { .. }) => {
                    // a singleton string type
                    self.bump();
                    Ok(())
                }
                Some(TokKind::Ident) if !is_reserved(self.text()) => {
                    self.bump();
                    if self.at(".") {
                        self.bump();
                        self.expect_name()?;
                    }
                    if self.at("<") {
                        self.angle_span()?;
                    }
                    // a generic type pack, `T...`
                    if self.at("...") {
                        self.bump();
                    }
                    Ok(())
                }
                _ => Err(self.err(&format!("expected a type, found {}", self.found()))),
            },
        }
    }

    fn type_table(&mut self) -> Result<(), ParseError> {
        self.expect("{")?;
        while !self.at("}") {
            if self.at_end() {
                return Err(self.err("unterminated table type"));
            }
            if self.at("[") {
                self.bump();
                self.type_body()?;
                self.expect("]")?;
                self.expect(":")?;
                self.type_body()?;
            } else {
                // `read`/`write` access modifiers come before the name
                if matches!(self.text(), "read" | "write")
                    && matches!(self.kind_at(1), Some(TokKind::Ident))
                {
                    self.bump();
                }
                if self.at_name() && self.text_at(1) == ":" {
                    self.bump();
                    self.bump();
                    self.type_body()?;
                } else {
                    // an array style element
                    self.type_body()?;
                }
            }
            if !self.eat(",") && !self.eat(";") {
                break;
            }
        }
        self.expect("}")?;
        Ok(())
    }
}

fn is_reserved(word: &str) -> bool {
    matches!(
        word,
        "and"
            | "break"
            | "do"
            | "else"
            | "elseif"
            | "end"
            | "false"
            | "for"
            | "function"
            | "if"
            | "in"
            | "local"
            | "nil"
            | "not"
            | "or"
            | "repeat"
            | "return"
            | "then"
            | "true"
            | "until"
            | "while"
    )
}

fn is_unary_op(s: &str) -> bool {
    matches!(s, "not" | "-" | "#")
}

fn is_compound_op(s: &str) -> bool {
    matches!(s, "+=" | "-=" | "*=" | "/=" | "%=" | "^=" | "..=" | "//=")
}

/// Left and right binding power, right lower than left means right associative
fn binop_priority(s: &str) -> Option<(u8, u8)> {
    Some(match s {
        "or" => (1, 1),
        "and" => (2, 2),
        "<" | ">" | "<=" | ">=" | "~=" | "==" => (3, 3),
        ".." => (9, 8),
        "+" | "-" => (10, 10),
        "*" | "/" | "//" | "%" => (11, 11),
        "^" => (14, 13),
        _ => return None,
    })
}
