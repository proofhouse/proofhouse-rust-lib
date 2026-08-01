// SPDX-License-Identifier: Apache-2.0
// Copyright Authors of Proofhouse

//! Evaluation that hands independent subtrees to worker threads.

use crate::ast::Expr;
use crate::cache::ExprCache;
use crate::errors::EvalError;
use crate::evaluator::{Rational, apply};
use std::thread;

/// Reduce an expression tree to its exact rational value, taking the
/// two sides of an infix operator in parallel.
///
/// The operands of an infix node share no data, which is what makes
/// them safe to reduce apart. Their values meet again in a table the
/// workers share, so a subexpression written twice costs one
/// reduction wherever the walk reaches it, and the answer matches what
/// [`crate::evaluator::evaluate`] gives for the same tree.
///
/// # Errors
///
/// Returns [`EvalError::DivisionByZero`] or [`EvalError::Overflow`] on
/// the same trees the ordinary walk reports them for. A failure on
/// either side of an operator is the failure of the node.
pub fn evaluate_parallel(expr: &Expr) -> Result<Rational, EvalError> {
    evaluate_shared(expr, &ExprCache::new())
}

/// Reduce `expr` against a table other threads are reducing into.
fn evaluate_shared(expr: &Expr, cache: &ExprCache) -> Result<Rational, EvalError> {
    match expr {
        // A literal is already a value and a chain of prefix minuses
        // is one operand deep, so neither has two sides to take apart.
        Expr::Number(_) | Expr::UnaryOp { .. } => cache.evaluate(expr),
        Expr::BinaryOp { op, left, right } => cache.get_or_compute(expr, || {
            thread::scope(|scope| {
                scope.spawn(|| evaluate_shared(left, cache));
                // Whatever this side reduces to reaches the table by
                // the time the scope closes, and the line that follows
                // reads both sides back out of it. Holding the value
                // here as well would say the same thing twice.
                let _shared = evaluate_shared(right, cache);
            });
            apply(*op, cache.evaluate(left)?, cache.evaluate(right)?)
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::evaluate_parallel;
    use crate::ast::{BinaryOperator, Expr};
    use crate::errors::EvalError;
    use crate::evaluator::evaluate;
    use crate::parser::parse;

    /// Texts covering each node type, nesting on both sides of an
    /// operator, a subexpression repeated within one tree, the two ways
    /// a walk fails, and a failure arriving from either operand of a
    /// node whose own operator would have succeeded.
    const CASES: &[&str] = &[
        "7",
        "-3",
        "---3",
        "2 + 3",
        "1/3 + 1/6",
        "(1 + 2) * (3 - 4)",
        "9 - 5 - 2",
        "(2 + 3) * (2 + 3)",
        "-(1 + 2 * 3) / (4 - 1)",
        "1/0",
        "(1 + 2) / (3 - 3)",
        "1/0 + 1",
        "1 + 1/0",
    ];

    #[test]
    fn the_parallel_walk_agrees_with_the_serial_one() {
        for text in CASES {
            let expr = parse(text).unwrap();
            assert_eq!(evaluate_parallel(&expr), evaluate(&expr), "text {text:?}");
        }
    }

    #[test]
    fn an_overflow_on_one_side_fails_the_node() {
        let expr = Expr::BinaryOp {
            op: BinaryOperator::Add,
            left: Box::new(Expr::Number(i64::MAX)),
            right: Box::new(Expr::Number(i64::MAX)),
        };
        assert_eq!(evaluate_parallel(&expr), Err(EvalError::Overflow));
    }
}
