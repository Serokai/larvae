//! Instance expression requires read as input, ex: require(script.Parent.Foo)

use crate::diag::Diag;
use crate::syntax::scan::{InstanceSite, Root, Step};

use super::*;

impl Resolver<'_> {
    /*
    Turn an instance chain into whatever the configured target wants

    The chain gives a datamodel path, the mount table turns that back into a
    file, and from there it is the same emission every other require goes
    through, so realm checks and container rules come along for free. None
    means leave the expression exactly as it was, which is always the safe
    answer when we cannot follow the chain all the way
    */
    pub fn resolve_instance(
        &self,
        ctx: &FileCtx,
        site: &InstanceSite,
        src: &str,
        diags: &mut Vec<Diag>,
    ) -> Option<String> {
        let offset = site.at as usize;
        let shown = site.render();

        let mut segments = match site.root {
            Root::Game => Vec::new(),

            Root::Script => {
                let Some(dm) = &ctx.dm else {
                    diags.push(
                        Diag::warning(
                            ctx.path,
                            format!(
                                "require({shown}) is relative to this script but nothing mounts this file into the DataModel, leaving it alone"
                            ),
                        )
                        .at(src, offset),
                    );

                    return None;
                };

                dm.segments.clone()
            }
        };

        for step in &site.steps {
            match step {
                Step::Up => {
                    if segments.pop().is_none() {
                        diags.push(
                            Diag::error(
                                ctx.path,
                                format!("require({shown}) walks above the DataModel root"),
                            )
                            .at(src, offset),
                        );

                        return None;
                    }
                }

                Step::Down(name) => segments.push(name.clone()),
            }
        }

        if segments.is_empty() {
            diags.push(
                Diag::error(
                    ctx.path,
                    format!("require({shown}) resolves to the DataModel itself"),
                )
                .at(src, offset),
            );

            return None;
        }

        let Some(target_base) = self.mounts.fs_of(&segments) else {
            diags.push(
                Diag::warning(
                    ctx.path,
                    format!(
                        "require({shown}) points at @game/{} and nothing in the project maps there, leaving it alone",
                        segments.join("/")
                    ),
                )
                .at(src, offset),
            );

            return None;
        };

        /*
        Emission writes specs back as require("..."), which reads wrong for a
        chain that never had quotes, so its diagnostics get unwrapped on the
        way out. Everything else about the emission is shared on purpose
        */
        let mut emitted = Vec::new();
        let rewrite = self.emit_fs(ctx, &shown, &target_base, src, offset, &mut emitted);
        let quoted = format!("require(\"{shown}\")");
        let plain = format!("require({shown})");

        for mut d in emitted {
            d.message = d.message.replace(&quoted, &plain);
            diags.push(d);
        }

        match rewrite {
            // a string spec has to come back with quotes, it is replacing an expression
            Rewrite::Replace(spec) => Some(lua_quote(&spec, self.quote)),

            Rewrite::Expr(expr) => Some(expr),

            Rewrite::Keep => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::syntax::lexer::lex;
    use crate::syntax::scan::{Root, Step, scan};

    fn chain(src: &str) -> Option<(Root, Vec<Step>)> {
        let toks = lex(src).unwrap().toks;
        let found = scan(src, &toks);

        found.instances.first().map(|s| (s.root, s.steps.clone()))
    }

    #[test]
    fn reads_the_common_shapes() {
        assert_eq!(
            chain("require(script.Parent.Foo)").unwrap(),
            (Root::Script, vec![Step::Up, Step::Down("Foo".into())])
        );
        assert_eq!(
            chain("require(game.ReplicatedStorage.Packages.Signal)").unwrap(),
            (
                Root::Game,
                vec![
                    Step::Down("ReplicatedStorage".into()),
                    Step::Down("Packages".into()),
                    Step::Down("Signal".into()),
                ]
            )
        );
        assert_eq!(
            chain(r#"require(game:GetService("ReplicatedStorage").Shared)"#).unwrap(),
            (
                Root::Game,
                vec![
                    Step::Down("ReplicatedStorage".into()),
                    Step::Down("Shared".into())
                ]
            )
        );
        assert_eq!(
            chain(r#"require(script.Parent:WaitForChild("Thing"))"#).unwrap(),
            (Root::Script, vec![Step::Up, Step::Down("Thing".into())])
        );
        // the shape a wally link module generates
        assert_eq!(
            chain(r#"require(script.Parent._Index["sleitnick_signal@2.0.1"]["signal"])"#).unwrap(),
            (
                Root::Script,
                vec![
                    Step::Up,
                    Step::Down("_Index".into()),
                    Step::Down("sleitnick_signal@2.0.1".into()),
                    Step::Down("signal".into()),
                ]
            )
        );
    }

    #[test]
    fn leaves_anything_it_cannot_follow() {
        // a local standing in for a service, we do not track locals yet
        assert!(chain("require(ReplicatedStorage.Packages.Signal)").is_none());
        // a computed child
        assert!(chain("require(script.Parent[name])").is_none());
        // a call we do not model
        assert!(chain("require(script.Parent:FindFirstAncestor(\"x\"))").is_none());
        // bare script with no hops
        assert!(chain("require(script)").is_none());
        // trailing work after the chain
        assert!(chain("require(script.Parent.Foo.Bar())").is_none());
    }
}
