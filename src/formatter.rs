// SPDX-License-Identifier: Apache-2.0
// Copyright Authors of Proofhouse

//! Canonical text rendering of an expression tree.

use crate::ast::{BinaryOperator, Expr, UnaryOperator};

/// Binding power of `+` and `-` as infix operators, the loosest the
/// grammar has.
const ADDITIVE: u8 = 1;

/// Binding power of `*` and `/`, one step over the additive pair.
const MULTIPLICATIVE: u8 = 2;

/// Binding power of prefix `-`, which reads a single operand rather
/// than a whole expression and so holds on tighter than any infix
/// operator.
const UNARY: u8 = 3;

/// Binding power of a bare literal, which no operator around it can
/// take apart.
const ATOM: u8 = 4;

/// Render an expression tree as text that parses back to the same tree.
///
/// An infix operator takes a space on either side and a prefix `-` none
/// after it. Parentheses appear only where dropping them would let a
/// looser operator outside capture an operand, or let a left-grouping
/// chain regroup. Everywhere else the text stays bare, which leaves one
/// tree with one spelling and makes two renderings equal exactly when
/// the trees behind them are.
#[must_use]
pub fn format_expr(expr: &Expr) -> String {
    match expr {
        Expr::Number(value) => value.to_string(),
        Expr::UnaryOp { op, operand } => match op {
            // Negation is the one prefix operator, and the operand
            // beside it has to hold on at least as tightly as the
            // minus does.
            UnaryOperator::Neg => format!("-{}", guarded(operand, UNARY)),
        },
        Expr::BinaryOp { op, left, right } => {
            let (own, right_floor) = infix_powers(*op);
            format!(
                "{} {} {}",
                guarded(left, own),
                binary_symbol(*op),
                guarded(right, right_floor)
            )
        }
    }
}

/// The power an infix operator reads at, paired with the power its
/// right operand has to reach.
///
/// A left operand carries the operator's own power and needs no guard
/// at that height, reading grouping to the left being what the parser
/// does already. A right operand answers to the level over it:
/// `9 - 5 - 2` groups as `(9 - 5) - 2`, so a subtraction on the right of
/// another one keeps its shape only inside parentheses.
const fn infix_powers(op: BinaryOperator) -> (u8, u8) {
    match op {
        BinaryOperator::Add | BinaryOperator::Sub => (ADDITIVE, MULTIPLICATIVE),
        BinaryOperator::Mul | BinaryOperator::Div => (MULTIPLICATIVE, UNARY),
    }
}

/// The source character a binary operator stands for.
///
/// One mapping owns how an operator spells out, so no second copy of
/// the table can drift away from this one.
const fn binary_symbol(op: BinaryOperator) -> char {
    match op {
        BinaryOperator::Add => '+',
        BinaryOperator::Sub => '-',
        BinaryOperator::Mul => '*',
        BinaryOperator::Div => '/',
    }
}

/// Render a child node, wrapping it in parentheses when it holds on
/// less tightly than its position demands.
fn guarded(child: &Expr, floor: u8) -> String {
    let text = format_expr(child);
    if binding_power(child) < floor {
        format!("({text})")
    } else {
        text
    }
}

/// How tightly a node holds on to its own operands, read on the scale
/// the four constants set out.
const fn binding_power(expr: &Expr) -> u8 {
    match expr {
        Expr::Number(_) => ATOM,
        Expr::UnaryOp { .. } => UNARY,
        Expr::BinaryOp { op, .. } => infix_powers(*op).0,
    }
}

#[cfg(test)]
mod tests {
    use super::format_expr;
    use crate::ast::{BinaryOperator, Expr, UnaryOperator};
    use crate::parser::parse;

    /// Lift an integer into a leaf node.
    fn number(value: i64) -> Expr {
        Expr::Number(value)
    }

    /// Wrap an operand in a prefix minus.
    fn neg(operand: Expr) -> Expr {
        Expr::UnaryOp {
            op: UnaryOperator::Neg,
            operand: Box::new(operand),
        }
    }

    /// Join two operands under an infix operator.
    fn infix(op: BinaryOperator, left: Expr, right: Expr) -> Expr {
        Expr::BinaryOp {
            op,
            left: Box::new(left),
            right: Box::new(right),
        }
    }

    /// Join two operands under `+`.
    fn add(left: Expr, right: Expr) -> Expr {
        infix(BinaryOperator::Add, left, right)
    }

    /// Join two operands under `-`.
    fn sub(left: Expr, right: Expr) -> Expr {
        infix(BinaryOperator::Sub, left, right)
    }

    /// Join two operands under `*`.
    fn mul(left: Expr, right: Expr) -> Expr {
        infix(BinaryOperator::Mul, left, right)
    }

    /// Join two operands under `/`.
    fn div(left: Expr, right: Expr) -> Expr {
        infix(BinaryOperator::Div, left, right)
    }

    /// Trees paired with the one text each of them renders as.
    ///
    /// Both tests below read this table: the first checks the spelling,
    /// the second feeds every spelling back through the parser. Each
    /// tree is one the parser itself can build, which is what makes the
    /// second reading a round trip rather than a comparison against a
    /// shape no text produces.
    fn cases() -> Vec<(Expr, &'static str)> {
        vec![
            (number(7), "7"),
            (add(number(1), number(2)), "1 + 2"),
            (sub(number(9), number(5)), "9 - 5"),
            (mul(number(6), number(7)), "6 * 7"),
            (div(number(8), number(2)), "8 / 2"),
            (neg(number(3)), "-3"),
            (neg(neg(number(3))), "--3"),
            // A looser operator underneath a tighter one keeps its
            // parentheses.
            (mul(add(number(1), number(2)), number(3)), "(1 + 2) * 3"),
            (neg(add(number(1), number(2))), "-(1 + 2)"),
            // A tighter one underneath a looser one drops them.
            (add(number(1), mul(number(2), number(3))), "1 + 2 * 3"),
            (sub(number(8), div(number(4), number(2))), "8 - 4 / 2"),
            (mul(number(2), neg(number(3))), "2 * -3"),
            (mul(neg(number(3)), number(5)), "-3 * 5"),
            // Grouping to the left leaves a left operand bare and puts
            // a right one of the same power in parentheses.
            (sub(sub(number(9), number(5)), number(2)), "9 - 5 - 2"),
            (sub(number(9), sub(number(5), number(2))), "9 - (5 - 2)"),
            (add(number(1), add(number(2), number(3))), "1 + (2 + 3)"),
            (div(div(number(8), number(4)), number(2)), "8 / 4 / 2"),
            (div(number(8), div(number(4), number(2))), "8 / (4 / 2)"),
            (mul(div(number(8), number(2)), number(3)), "8 / 2 * 3"),
            (mul(number(2), mul(number(3), number(4))), "2 * (3 * 4)"),
            // Both operands guarded at once, and a mixed tree read
            // under a prefix minus.
            (
                mul(add(number(1), number(2)), sub(number(3), number(4))),
                "(1 + 2) * (3 - 4)",
            ),
            (
                neg(add(number(1), mul(number(2), number(3)))),
                "-(1 + 2 * 3)",
            ),
        ]
    }

    #[test]
    fn format_expr_spells_each_tree_one_way() {
        for (expr, expected) in cases() {
            assert_eq!(format_expr(&expr), expected, "tree {expr:?}");
        }
    }

    #[test]
    fn every_rendering_parses_back_to_its_own_tree() {
        for (expr, _) in cases() {
            let text = format_expr(&expr);
            assert_eq!(parse(&text).as_ref(), Ok(&expr), "text {text:?}");
        }
    }
}
