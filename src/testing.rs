// SPDX-License-Identifier: Apache-2.0
// Copyright Authors of Proofhouse

//! Input generators this library offers to property suites.
//!
//! Each function here builds a proptest strategy over one of the crate's
//! own data shapes, so a suite writes down the law it expects and leaves
//! the hunt for a counterexample to the generator. The module answers to
//! a feature, this being the one place the library names proptest at
//! all: a build that never asks for `testing` resolves without that
//! crate on the graph.

use crate::ast::{BinaryOperator, Expr, UnaryOperator};
use crate::formatter::format_expr;
use crate::tokens::{Token, TokenKind};
use proptest::collection;
use proptest::prop_oneof;
use proptest::strategy::{Just, Strategy};

/// Node count the tree strategy aims at before the depth bound stops
/// it, which is what decides how often a draw takes a branch over a
/// leaf.
const DESIRED_SIZE: u32 = 32;

/// Nodes one branching step adds, read against the preceding size.
const BRANCH_SIZE: u32 = 2;

/// Pair a kind with its text at the front of whatever spells it out.
///
/// Where a token sits depends on the run holding it, so the offset
/// waits for [`token_sequences`] and starts at zero until then.
const fn at_start(kind: TokenKind, lexeme: String) -> Token {
    Token {
        kind,
        lexeme,
        offset: 0,
    }
}

/// Draw one token the lexer could itself have produced.
///
/// A number carries a digit run the draw picks. Every operator and
/// bracket carries the single character it always spells as, taken from
/// the arms below rather than drawn beside the kind, which is what stops
/// a token pairing a kind with text the lexer reads some other way.
fn tokens() -> impl Strategy<Value = Token> {
    prop_oneof![
        // Nonnegative, a minus in the source being a token of its own
        // rather than part of the number after it.
        (0..=i64::MAX).prop_map(|value| at_start(TokenKind::Number, value.to_string())),
        Just(at_start(TokenKind::Plus, "+".to_owned())),
        Just(at_start(TokenKind::Minus, "-".to_owned())),
        Just(at_start(TokenKind::Star, "*".to_owned())),
        Just(at_start(TokenKind::Slash, "/".to_owned())),
        Just(at_start(TokenKind::LParen, "(".to_owned())),
        Just(at_start(TokenKind::RParen, ")".to_owned())),
    ]
}

/// Move each token of a run to the offset its own spelling puts it at.
#[expect(
    clippy::arithmetic_side_effects,
    reason = "the running total counts bytes of strings already in memory"
)]
fn placed(drawn: Vec<Token>) -> Vec<Token> {
    let mut offset = 0;
    let mut run = Vec::with_capacity(drawn.len());
    for token in drawn {
        let width = token.lexeme.len();
        run.push(Token { offset, ..token });
        // One for the separator [`spelling`] writes between neighbors.
        offset += width + 1;
    }
    run
}

/// Draw a run of up to `max_len` tokens, each at the offset its own
/// spelling gives it.
///
/// The offsets answer to [`spelling`], which sets one space between
/// neighbors so a pair of digit runs stays two tokens instead of
/// merging into one.
pub fn token_sequences(max_len: usize) -> impl Strategy<Value = Vec<Token>> {
    collection::vec(tokens(), 0..=max_len).prop_map(placed)
}

/// Spell a run of tokens out as the text it stands for.
///
/// A single space joins each pair, which is the spacing the offsets
/// from [`token_sequences`] count on.
#[must_use]
pub fn spelling(run: &[Token]) -> String {
    run.iter()
        .map(|token| token.lexeme.as_str())
        .collect::<Vec<&str>>()
        .join(" ")
}

/// Draw one of the four infix operators.
fn binary_operators() -> impl Strategy<Value = BinaryOperator> {
    prop_oneof![
        Just(BinaryOperator::Add),
        Just(BinaryOperator::Sub),
        Just(BinaryOperator::Mul),
        Just(BinaryOperator::Div),
    ]
}

/// Draw well-formed expression trees down to a bounded nesting depth.
///
/// A leaf holds a nonnegative literal, which is the only kind the parser
/// builds: a minus in the source becomes a prefix operator over the
/// number beside it rather than part of that number, so a negative
/// literal names a tree no text spells. A branch either applies that
/// prefix minus to one child or joins two children under an infix
/// operator, and `max_depth` caps how many branches stack before the
/// recursion has to bottom out at a leaf.
pub fn expressions(max_depth: u32) -> impl Strategy<Value = Expr> {
    (0..=i64::MAX).prop_map(Expr::Number).prop_recursive(
        max_depth,
        DESIRED_SIZE,
        BRANCH_SIZE,
        |inner| {
            prop_oneof![
                inner.clone().prop_map(|operand| Expr::UnaryOp {
                    op: UnaryOperator::Neg,
                    operand: Box::new(operand),
                }),
                (binary_operators(), inner.clone(), inner).prop_map(|(op, left, right)| {
                    Expr::BinaryOp {
                        op,
                        left: Box::new(left),
                        right: Box::new(right),
                    }
                }),
            ]
        },
    )
}

/// Draw source text that parses back to the tree behind it.
///
/// Each string is the canonical rendering of a tree from
/// [`expressions`], which leaves it parseable by construction. A suite
/// after arbitrary valid input takes it from here rather than filtering
/// a character strategy down to the rare string the grammar accepts.
pub fn expression_texts(max_depth: u32) -> impl Strategy<Value = String> {
    expressions(max_depth).prop_map(|expr| format_expr(&expr))
}
