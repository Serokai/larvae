/*!
Every lint, grouped by the kind of mistake it catches.

The grouping is for reading, not for behaviour. A lint's name is what a user
configures and it is the same wherever the lint lives, so moving one between
these files changes nothing a project can see.
*/

pub mod correctness;
pub mod style;

use super::Lint;

/*
The registry.

Order decides only which finding is listed first when two land on the same
byte, so it follows severity of consequence: something that is wrong before
something that is merely untidy.
*/
pub static ALL: &[&dyn Lint] = &[
    // correctness
    &correctness::AlmostSwapped,
    &correctness::CompareNan,
    &correctness::ConstantTableComparison,
    &correctness::DivideByZero,
    &correctness::DuplicateKeys,
    &correctness::IfsSameCond,
    &correctness::IfSameThenElse,
    &correctness::SuspiciousReverseLoop,
    &correctness::TypeCheckInsideCall,
    &correctness::UnbalancedAssignments,
    // style
    &style::EmptyIf,
    &style::EmptyLoop,
    &style::MixedTable,
    &style::MultipleStatements,
    &style::ParentheseConditions,
];

/*
The boilerplate every lint shares.

Each entry declares the unit type, the name a user writes, whether it is on by
default, and the one line explanation. The lint itself is the `check` function,
written normally, so only the part that differs is written per lint.
*/
#[macro_export]
macro_rules! lints {
    ($($ty:ident => $name:literal, $level:ident, $about:literal;)*) => {
        $(
            pub struct $ty;

            impl $crate::lint::Lint for $ty {
                fn name(&self) -> &'static str {
                    $name
                }

                fn default_level(&self) -> $crate::lint::Level {
                    $crate::lint::Level::$level
                }

                fn about(&self) -> &'static str {
                    $about
                }

                fn run(
                    &self,
                    ctx: &$crate::lint::LintCtx<'_>,
                    out: &mut Vec<$crate::lint::Finding>,
                ) {
                    $ty::check(ctx, out)
                }
            }
        )*
    };
}
