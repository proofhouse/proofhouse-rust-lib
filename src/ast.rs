// SPDX-License-Identifier: Apache-2.0
// Copyright Authors of Proofhouse

//! Node types the parser builds an expression out of.

/// Operator a [`Expr::UnaryOp`] node applies to its operand.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnaryOperator {
    /// Arithmetic negation, written as a prefix `-`.
    Neg,
}

/// Operator a [`Expr::BinaryOp`] node applies to its operands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinaryOperator {
    /// Addition, written `+`.
    Add,
    /// Subtraction, written `-`.
    Sub,
    /// Multiplication, written `*`.
    Mul,
    /// Division, written `/`.
    Div,
}

/// One node of an expression tree.
///
/// Operands hang off their operator through a [`Box`], which is what
/// gives the type a finite size while letting a tree nest as deep as the
/// source text does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Expr {
    /// Integer literal leaf.
    Number(i64),
    /// Prefix operator applied to a single operand.
    UnaryOp {
        /// The operator this node applies.
        op: UnaryOperator,
        /// The expression the operator reads.
        operand: Box<Self>,
    },
    /// Infix operator applied to a left and a right operand.
    BinaryOp {
        /// The operator this node applies.
        op: BinaryOperator,
        /// The expression on the operator's left.
        left: Box<Self>,
        /// The expression on the operator's right.
        right: Box<Self>,
    },
}
