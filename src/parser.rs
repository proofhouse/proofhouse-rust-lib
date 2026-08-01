// SPDX-License-Identifier: Apache-2.0
// Copyright Authors of Proofhouse

//! Parser turning expression text into a tree.

use std::iter::Peekable;
use std::slice::Iter;

use crate::ast::{BinaryOperator, Expr, UnaryOperator};
use crate::errors::{ExpressionError, ParseError, ParseErrorKind};
use crate::lexer::tokenize;
use crate::tokens::{Token, TokenKind};

/// Precedence level of `+` and `-` as infix operators.
const ADDITIVE: u8 = 1;

/// Precedence level of `*` and `/`, which bind tighter than the additive
/// pair. Prefix `-` binds tighter still and needs no level of its own:
/// it takes an operand rather than a whole expression.
const MULTIPLICATIVE: u8 = 2;

/// Parse expression text into a tree.
///
/// Infix `+` and `-` bind loosest and group to the left, `*` and `/`
/// bind tighter and group the same way, and prefix `-` binds tighter
/// than either. Parentheses group whatever they enclose and leave no
/// node of their own behind. Whitespace separates tokens and means
/// nothing else.
///
/// Every literal has to fit an [`i64`] on its own, which leaves the
/// digits of the most negative such value out of range even where a
/// prefix `-` precedes them.
///
/// # Errors
///
/// Returns [`ExpressionError::Lex`] when the text holds a character no
/// token starts with, and [`ExpressionError::Parse`] when the tokens
/// form no expression. Either error names the byte offset it refers to.
pub fn parse(text: &str) -> Result<Expr, ExpressionError> {
    let tokens = tokenize(text)?;
    let mut parser = Parser {
        tokens: tokens.iter().peekable(),
        end_offset: text.len(),
    };
    let expr = parser.parse_expression(ADDITIVE)?;
    parser.check_exhausted()?;
    Ok(expr)
}

/// Precedence-climbing parser reading a token stream once, left to
/// right.
struct Parser<'tokens> {
    /// Tokens still unread, oldest first.
    tokens: Peekable<Iter<'tokens, Token>>,
    /// Byte offset one past the end of the source text, which is what an
    /// error reports when the stream runs out.
    end_offset: usize,
}

impl<'tokens> Parser<'tokens> {
    /// Parse a chain of infix operators binding at least as tightly as
    /// `min_precedence`, stopping at the first one that binds looser.
    fn parse_expression(&mut self, min_precedence: u8) -> Result<Expr, ParseError> {
        let mut left = self.parse_operand()?;
        while let Some((op, precedence)) = self.take_infix_operator(min_precedence) {
            // Climbing one level for the right operand is what makes
            // operators of equal precedence group to the left: the
            // recursive call refuses to swallow the next one.
            let right = self.parse_expression(precedence.saturating_add(1))?;
            left = Expr::BinaryOp {
                op,
                left: Box::new(left),
                right: Box::new(right),
            };
        }
        Ok(left)
    }

    /// Parse a literal, a prefix `-` applied to another operand, or a
    /// parenthesized expression.
    fn parse_operand(&mut self) -> Result<Expr, ParseError> {
        let token = self.take()?;
        match token.kind {
            TokenKind::Number => number(token),
            TokenKind::Minus => Ok(Expr::UnaryOp {
                op: UnaryOperator::Neg,
                operand: Box::new(self.parse_operand()?),
            }),
            TokenKind::LParen => {
                let inner = self.parse_expression(ADDITIVE)?;
                let closing = self.take()?;
                match closing.kind {
                    TokenKind::RParen => Ok(inner),
                    TokenKind::Number
                    | TokenKind::Plus
                    | TokenKind::Minus
                    | TokenKind::Star
                    | TokenKind::Slash
                    | TokenKind::LParen => Err(unexpected(closing)),
                }
            }
            TokenKind::Plus | TokenKind::Star | TokenKind::Slash | TokenKind::RParen => {
                Err(unexpected(token))
            }
        }
    }

    /// Fail when any token follows the expression just parsed.
    fn check_exhausted(&mut self) -> Result<(), ParseError> {
        self.tokens.next().map_or(Ok(()), |token| {
            Err(ParseError {
                offset: token.offset,
                kind: ParseErrorKind::TrailingInput,
            })
        })
    }

    /// Consume the next token when it stands for an infix operator that
    /// binds at least as tightly as `min_precedence`, and report that
    /// operator along with its level.
    fn take_infix_operator(&mut self, min_precedence: u8) -> Option<(BinaryOperator, u8)> {
        let token = self.tokens.next_if(|token| {
            infix_operator(token.kind).is_some_and(|(_, precedence)| precedence >= min_precedence)
        })?;
        infix_operator(token.kind)
    }

    /// Consume the next token, or fail at the end of the input.
    fn take(&mut self) -> Result<&'tokens Token, ParseError> {
        self.tokens.next().ok_or(ParseError {
            offset: self.end_offset,
            kind: ParseErrorKind::UnexpectedEndOfInput,
        })
    }
}

/// Report the operator a token kind stands for as an infix operator,
/// together with how tightly it binds.
const fn infix_operator(kind: TokenKind) -> Option<(BinaryOperator, u8)> {
    match kind {
        TokenKind::Plus => Some((BinaryOperator::Add, ADDITIVE)),
        TokenKind::Minus => Some((BinaryOperator::Sub, ADDITIVE)),
        TokenKind::Star => Some((BinaryOperator::Mul, MULTIPLICATIVE)),
        TokenKind::Slash => Some((BinaryOperator::Div, MULTIPLICATIVE)),
        TokenKind::Number | TokenKind::LParen | TokenKind::RParen => None,
    }
}

/// Read a number token's digits as an [`i64`] leaf. The lexer hands over
/// a run of digits and nothing else, so overflow is the one way the
/// conversion fails and the discarded error carries no news.
fn number(token: &Token) -> Result<Expr, ParseError> {
    token
        .lexeme
        .parse::<i64>()
        .ok()
        .map(Expr::Number)
        .ok_or(ParseError {
            offset: token.offset,
            kind: ParseErrorKind::NumberOutOfRange,
        })
}

/// Build the error for a token the grammar has no use for here.
const fn unexpected(token: &Token) -> ParseError {
    ParseError {
        offset: token.offset,
        kind: ParseErrorKind::UnexpectedToken,
    }
}

#[cfg(test)]
mod tests {
    use super::parse;
    use crate::ast::{BinaryOperator, Expr, UnaryOperator};
    use crate::errors::{ExpressionError, LexError, ParseError, ParseErrorKind};

    /// Render a tree with a pair of parentheses around every operator
    /// node, so precedence and grouping read straight off the string
    /// instead of hiding in whatever a minimal-parenthesis form drops.
    fn render(expr: &Expr) -> String {
        match expr {
            Expr::Number(value) => value.to_string(),
            Expr::UnaryOp { op, operand } => match op {
                UnaryOperator::Neg => format!("(-{})", render(operand)),
            },
            Expr::BinaryOp { op, left, right } => {
                let symbol = match op {
                    BinaryOperator::Add => '+',
                    BinaryOperator::Sub => '-',
                    BinaryOperator::Mul => '*',
                    BinaryOperator::Div => '/',
                };
                format!("({} {} {})", render(left), symbol, render(right))
            }
        }
    }

    #[test]
    fn parse_builds_expected_nodes() {
        let cases: &[(&str, Expr)] = &[
            ("7", Expr::Number(7)),
            (
                "1+2",
                Expr::BinaryOp {
                    op: BinaryOperator::Add,
                    left: Box::new(Expr::Number(1)),
                    right: Box::new(Expr::Number(2)),
                },
            ),
            (
                "-3",
                Expr::UnaryOp {
                    op: UnaryOperator::Neg,
                    operand: Box::new(Expr::Number(3)),
                },
            ),
            ("(7)", Expr::Number(7)),
        ];
        for (input, expected) in cases {
            assert_eq!(parse(input).as_ref(), Ok(expected), "input {input:?}");
        }
    }

    #[test]
    fn parse_shapes_precedence_and_associativity() {
        let cases: &[(&str, &str)] = &[
            ("1+2*3", "(1 + (2 * 3))"),
            ("1*2+3", "((1 * 2) + 3)"),
            ("8-4/2", "(8 - (4 / 2))"),
            ("1+2+3", "((1 + 2) + 3)"),
            ("9-5-2", "((9 - 5) - 2)"),
            ("8/4/2", "((8 / 4) / 2)"),
            ("2*3*4", "((2 * 3) * 4)"),
            ("2*3/4", "((2 * 3) / 4)"),
            ("8/2*3", "((8 / 2) * 3)"),
            ("(1+2)*3", "((1 + 2) * 3)"),
            ("2*(3-(4+5))", "(2 * (3 - (4 + 5)))"),
            ("((7))", "7"),
            ("- -3", "(-(-3))"),
            ("---3", "(-(-(-3)))"),
            ("-2*3", "((-2) * 3)"),
            ("2*-3", "(2 * (-3))"),
            ("-(1+2)", "(-(1 + 2))"),
            (" 1 + 2 ", "(1 + 2)"),
        ];
        for &(input, expected) in cases {
            let expr = parse(input).unwrap();
            assert_eq!(render(&expr), expected, "input {input:?}");
        }
    }

    #[test]
    fn parse_reads_the_largest_literal_that_fits() {
        assert_eq!(parse("9223372036854775807"), Ok(Expr::Number(i64::MAX)));
    }

    #[test]
    fn parse_rejects_bad_syntax_with_kind_and_offset() {
        let cases: &[(&str, ParseErrorKind, usize)] = &[
            ("", ParseErrorKind::UnexpectedEndOfInput, 0),
            ("   ", ParseErrorKind::UnexpectedEndOfInput, 3),
            ("1+", ParseErrorKind::UnexpectedEndOfInput, 2),
            ("-", ParseErrorKind::UnexpectedEndOfInput, 1),
            ("(1+2", ParseErrorKind::UnexpectedEndOfInput, 4),
            ("*3", ParseErrorKind::UnexpectedToken, 0),
            ("1+*2", ParseErrorKind::UnexpectedToken, 2),
            ("()", ParseErrorKind::UnexpectedToken, 1),
            ("(1 2", ParseErrorKind::UnexpectedToken, 3),
            ("1+2)", ParseErrorKind::TrailingInput, 3),
            ("1 2", ParseErrorKind::TrailingInput, 2),
            ("(1)(2)", ParseErrorKind::TrailingInput, 3),
            // One past the largest value that fits, first on its own
            // and then behind a prefix minus. The parser weighs the
            // digits alone, so the sign in front buys them no room.
            ("9223372036854775808", ParseErrorKind::NumberOutOfRange, 0),
            ("-9223372036854775808", ParseErrorKind::NumberOutOfRange, 1),
            (
                "1 + 99999999999999999999999",
                ParseErrorKind::NumberOutOfRange,
                4,
            ),
        ];
        for &(input, kind, offset) in cases {
            assert_eq!(
                parse(input),
                Err(ExpressionError::Parse(ParseError { offset, kind })),
                "input {input:?}"
            );
        }
    }

    #[test]
    fn parse_reports_a_lex_error_from_the_text() {
        assert_eq!(
            parse("1 $ 2"),
            Err(ExpressionError::Lex(LexError {
                offset: 2,
                character: '$'
            }))
        );
    }
}
