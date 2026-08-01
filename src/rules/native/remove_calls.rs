/*!
remove_calls, drop statement position calls to named functions, ex: print,
warn, debug.profilebegin, a call whose value is used is never touched since
removing it would corrupt the expression around it
*/

use crate::config::RemoveCalls;
use crate::rules::engine::{self, Edit, RuleCtx, Visit};
use crate::rules::native::{blank_line_start, contains_call, dotted_path};
use crate::syntax::ast::{CallArgs, Expr, Stmt};

pub fn apply(cfg: &RemoveCalls, ctx: &RuleCtx, edits: &mut Vec<Edit>) {
    let mut remover = Remover {
        ctx,
        names: cfg.functions(),
        preserve: cfg.preserve_arguments_side_effects(),
        edits,
    };

    engine::walk_chunk(ctx.chunk, &mut remover);
}

struct Remover<'a, 'src> {
    ctx: &'a RuleCtx<'src>,
    names: &'a [String],
    preserve: bool,
    edits: &'a mut Vec<Edit>,
}

impl Visit for Remover<'_, '_> {
    fn stmt(&mut self, stmt: &Stmt) {
        let Stmt::Call(call, span) = stmt else {
            return;
        };

        let Expr::Call {
            func,
            method: None,
            args,
            ..
        } = call
        else {
            // a method call has a receiver, matching it by name would be a guess
            return;
        };

        // plain Name or Name.Name chains only, a computed index is not a name
        let Some(path) = dotted_path(self.ctx, func) else {
            return;
        };

        if !self.names.contains(&path) {
            return;
        }

        if self.preserve && args_may_do_work(args) {
            return;
        }

        let (start, end) = self.ctx.bytes(*span);
        // eat the indent too so the line does not keep trailing blanks
        let from = blank_line_start(self.ctx, start);
        self.ctx.delete_bytes_keep_lines(from, end, self.edits);
    }
}

/// True when an argument calls something, that call may be the point of the line
fn args_may_do_work(args: &CallArgs) -> bool {
    match args {
        CallArgs::Paren(list) => list.iter().any(contains_call),

        CallArgs::Table(e) => contains_call(e),

        CallArgs::Str(_) => false,
    }
}

#[cfg(test)]
mod tests {
    use crate::rules::native::test_support::run;

    const PRINTS: &str = "remove_calls = [\"print\", \"warn\", \"debug.profilebegin\"]";

    #[test]
    fn removes_statement_calls() {
        assert_eq!(run(PRINTS, "print(\"hi\")\nreturn 1\n"), "\nreturn 1\n");
        assert_eq!(run(PRINTS, "warn(\"hi\")\n"), "\n");
        // dotted paths and indented lines
        assert_eq!(
            run(PRINTS, "do\n    debug.profilebegin(\"x\")\nend\n"),
            "do\n\nend\n"
        );
        // multi line calls keep their line count
        assert_eq!(
            run(PRINTS, "print(\n  1\n)\nreturn 2\n"),
            "\n\n\nreturn 2\n"
        );
    }

    #[test]
    fn leaves_used_values_and_unlisted_names_alone() {
        // the value is used, removing the call would corrupt the statement
        let used = "local x = print(\"a\")\n";
        assert_eq!(run(PRINTS, used), used);
        // method calls have a receiver, the name alone proves nothing
        let method = "obj:print(\"a\")\n";
        assert_eq!(run(PRINTS, method), method);
        // computed callees are not names
        let computed = "t[\"print\"](\"a\")\n";
        assert_eq!(run(PRINTS, computed), computed);
        // not on the list
        let other = "log(\"a\")\n";
        assert_eq!(run(PRINTS, other), other);
    }

    #[test]
    fn argument_side_effects_decide() {
        let src = "print(compute())\n";
        // the default keeps the call, compute() may be the point of the line
        assert_eq!(run(PRINTS, src), src);
        assert_eq!(
            run(
                "remove_calls = { functions = [\"print\"], preserve_arguments_side_effects = false }",
                src
            ),
            "\n"
        );
        // an argument without calls is removed either way
        assert_eq!(run(PRINTS, "print(x, 1 + 2)\n"), "\n");
    }
}
