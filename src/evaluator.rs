// SPDX-License-Identifier: Apache-2.0
// Copyright Authors of Proofhouse

//! Exact-arithmetic evaluator walking an expression tree.

use crate::ast::{BinaryOperator, Expr, UnaryOperator};
use crate::errors::{EvalError, ExpressionError};
use crate::parser::parse;

/// An exact rational number, kept as a numerator over a positive
/// denominator with no factor left in common.
///
/// The python sibling of this library reaches into its standard library
/// for a fraction type. Rust offers none, and a dependency bought for a
/// domain this small would outweigh the arithmetic it carries, so the
/// value type lives beside the evaluator that produces it.
///
/// Every value holds the one shape: a denominator greater than zero, a
/// sign riding on the numerator, and no factor left in both halves.
/// Comparing two values compares the numbers themselves, each number
/// having only the one spelling here.
///
/// Both halves are an [`i64`], so a computation can outgrow the type.
/// Each step that could reports [`EvalError::Overflow`] instead of
/// wrapping or panicking.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rational {
    /// Numerator of the reduced pair, carrying the sign of the value.
    numerator: i64,
    /// Denominator of the reduced pair, always greater than zero.
    denominator: i64,
}

impl Rational {
    /// Lift an integer to the rational that equals it.
    #[must_use]
    pub const fn from_integer(value: i64) -> Self {
        Self {
            numerator: value,
            denominator: 1,
        }
    }

    /// Reduce a numerator and a denominator to the one shape this type
    /// holds.
    ///
    /// # Errors
    ///
    /// Returns [`EvalError::DivisionByZero`] when the denominator is
    /// zero, and [`EvalError::Overflow`] when moving the sign off a
    /// negative denominator takes either half past the range of an
    /// [`i64`].
    pub fn new(numerator: i64, denominator: i64) -> Result<Self, EvalError> {
        if denominator == 0 {
            return Err(EvalError::DivisionByZero);
        }
        // The sign belongs to the numerator, so a negative denominator
        // hands it over. Negation is where the most negative value of
        // the type has no counterpart to become.
        let (signed, positive) = if denominator < 0 {
            (
                numerator.checked_neg().ok_or(EvalError::Overflow)?,
                denominator.checked_neg().ok_or(EvalError::Overflow)?,
            )
        } else {
            (numerator, denominator)
        };
        let divisor = greatest_common_divisor(signed, positive);
        Ok(Self::reduced(signed, positive, divisor))
    }

    /// The numerator of the reduced pair.
    #[must_use]
    pub const fn numerator(self) -> i64 {
        self.numerator
    }

    /// The denominator of the reduced pair.
    #[must_use]
    pub const fn denominator(self) -> i64 {
        self.denominator
    }

    /// Add two values, exactly.
    ///
    /// # Errors
    ///
    /// Returns [`EvalError::Overflow`] when a crossed product or their
    /// sum leaves the range of an [`i64`]. Crossing before reducing
    /// means the report can arrive for a sum whose reduced form would
    /// have fit.
    pub fn checked_add(self, other: Self) -> Result<Self, EvalError> {
        self.combine(other, i64::checked_add)
    }

    /// Subtract one value from another, exactly.
    ///
    /// # Errors
    ///
    /// Returns [`EvalError::Overflow`] on the same terms as
    /// [`Self::checked_add`], the two differing only in what they do
    /// with the crossed pair.
    pub fn checked_sub(self, other: Self) -> Result<Self, EvalError> {
        self.combine(other, i64::checked_sub)
    }

    /// Multiply two values, exactly.
    ///
    /// # Errors
    ///
    /// Returns [`EvalError::Overflow`] when either product leaves the
    /// range of an [`i64`].
    pub fn checked_mul(self, other: Self) -> Result<Self, EvalError> {
        let numerator = self
            .numerator
            .checked_mul(other.numerator)
            .ok_or(EvalError::Overflow)?;
        let denominator = self
            .denominator
            .checked_mul(other.denominator)
            .ok_or(EvalError::Overflow)?;
        Self::new(numerator, denominator)
    }

    /// Divide one value by another, exactly.
    ///
    /// # Errors
    ///
    /// Returns [`EvalError::DivisionByZero`] when the divisor is zero,
    /// and [`EvalError::Overflow`] when either product of the flipped
    /// divisor leaves the range of an [`i64`].
    pub fn checked_div(self, other: Self) -> Result<Self, EvalError> {
        if other.numerator == 0 {
            return Err(EvalError::DivisionByZero);
        }
        let numerator = self
            .numerator
            .checked_mul(other.denominator)
            .ok_or(EvalError::Overflow)?;
        let denominator = self
            .denominator
            .checked_mul(other.numerator)
            .ok_or(EvalError::Overflow)?;
        Self::new(numerator, denominator)
    }

    /// Flip the sign of a value.
    ///
    /// # Errors
    ///
    /// Returns [`EvalError::Overflow`] for the most negative numerator
    /// an [`i64`] holds, which has no positive counterpart to become.
    pub fn checked_neg(self) -> Result<Self, EvalError> {
        Ok(Self {
            numerator: self.numerator.checked_neg().ok_or(EvalError::Overflow)?,
            denominator: self.denominator,
        })
    }

    /// Cross-multiply two values and join the crossed numerators with
    /// `join`.
    ///
    /// A sum and a difference reach a common denominator by the same
    /// three products and part company at one step, which arrives here
    /// as a function rather than as a second copy of the surrounding
    /// arithmetic.
    fn combine(self, other: Self, join: fn(i64, i64) -> Option<i64>) -> Result<Self, EvalError> {
        let left = self
            .numerator
            .checked_mul(other.denominator)
            .ok_or(EvalError::Overflow)?;
        let right = other
            .numerator
            .checked_mul(self.denominator)
            .ok_or(EvalError::Overflow)?;
        let numerator = join(left, right).ok_or(EvalError::Overflow)?;
        let denominator = self
            .denominator
            .checked_mul(other.denominator)
            .ok_or(EvalError::Overflow)?;
        Self::new(numerator, denominator)
    }

    /// Divide a signed numerator and a positive denominator by a
    /// divisor that goes into both exactly.
    #[expect(
        clippy::arithmetic_side_effects,
        reason = "the divisor is at least one, so neither division can divide by zero and neither can overflow"
    )]
    #[expect(
        clippy::integer_division,
        clippy::integer_division_remainder_used,
        reason = "the divisor goes into both halves exactly, which is what reducing a fraction means, and nothing here is timing-sensitive"
    )]
    const fn reduced(numerator: i64, denominator: i64, divisor: i64) -> Self {
        Self {
            numerator: numerator / divisor,
            denominator: denominator / divisor,
        }
    }
}

/// The greatest common divisor of a numerator and a denominator known
/// to be greater than zero, reported as a positive value.
///
/// Euclid's algorithm, taking each step the euclidean way so what it
/// leaves behind never falls below zero whatever sign the numerator
/// carried. Every value the loop holds is then positive, which is what
/// leaves the answer positive with no sign work at the end.
const fn greatest_common_divisor(numerator: i64, denominator: i64) -> i64 {
    let mut larger = denominator;
    let mut smaller = numerator.rem_euclid(denominator);
    while smaller != 0 {
        let remainder = larger.rem_euclid(smaller);
        larger = smaller;
        smaller = remainder;
    }
    larger
}

/// Reduce an expression tree to its exact rational value.
///
/// Integer leaves lift to rationals, so the whole walk stays in exact
/// arithmetic and a division reduces where a machine division would
/// round.
///
/// # Errors
///
/// Returns [`EvalError::DivisionByZero`] when a divisor reduces to
/// zero, and [`EvalError::Overflow`] when a step leaves the range of an
/// [`i64`].
pub fn evaluate(expr: &Expr) -> Result<Rational, EvalError> {
    match expr {
        Expr::Number(value) => Ok(Rational::from_integer(*value)),
        Expr::UnaryOp { op, operand } => match op {
            // Negation is the one prefix operator and always flips the
            // sign of whatever it reads.
            UnaryOperator::Neg => evaluate(operand)?.checked_neg(),
        },
        Expr::BinaryOp { op, left, right } => apply(*op, evaluate(left)?, evaluate(right)?),
    }
}

/// Parse expression text and reduce it to its exact rational value.
///
/// # Errors
///
/// Returns whichever of [`ExpressionError::Lex`],
/// [`ExpressionError::Parse`], and [`ExpressionError::Eval`] the text
/// runs into first.
pub fn evaluate_text(text: &str) -> Result<Rational, ExpressionError> {
    evaluate(&parse(text)?).map_err(ExpressionError::from)
}

/// Apply an infix operator to a pair of values already reduced.
fn apply(op: BinaryOperator, left: Rational, right: Rational) -> Result<Rational, EvalError> {
    match op {
        BinaryOperator::Add => left.checked_add(right),
        BinaryOperator::Sub => left.checked_sub(right),
        BinaryOperator::Mul => left.checked_mul(right),
        BinaryOperator::Div => left.checked_div(right),
    }
}

#[cfg(test)]
mod tests {
    use super::{Rational, evaluate, evaluate_text};
    use crate::ast::{BinaryOperator, Expr, UnaryOperator};
    use crate::errors::{EvalError, ExpressionError, ParseError, ParseErrorKind};

    /// Build a value from a pair the case tables already know is valid.
    fn ratio(numerator: i64, denominator: i64) -> Rational {
        Rational::new(numerator, denominator).unwrap()
    }

    #[test]
    fn evaluate_text_yields_the_exact_value() {
        let cases: &[(&str, i64, i64)] = &[
            ("7", 7, 1),
            ("2+3", 5, 1),
            ("10-4", 6, 1),
            ("6*7", 42, 1),
            ("8/2", 4, 1),
            ("1/3", 1, 3),
            ("2/6", 1, 3),
            ("1/3+1/3+1/3", 1, 1),
            ("1+2*3", 7, 1),
            ("(1+2)*3", 9, 1),
            ("8-4/2", 6, 1),
            ("9-5-2", 2, 1),
            ("8/4/2", 1, 1),
            ("0/5", 0, 1),
            ("-3", -3, 1),
            ("- -3", 3, 1),
            ("---3", -3, 1),
            ("-(1+2)", -3, 1),
            ("2*-3", -6, 1),
            ("6/-2", -3, 1),
            ("6/-3", -2, 1),
            ("1/2-3/4", -1, 4),
        ];
        for &(input, numerator, denominator) in cases {
            assert_eq!(
                evaluate_text(input),
                Ok(ratio(numerator, denominator)),
                "input {input:?}"
            );
        }
    }

    #[test]
    fn evaluate_walks_a_prebuilt_tree() {
        let tree = Expr::BinaryOp {
            op: BinaryOperator::Mul,
            left: Box::new(Expr::UnaryOp {
                op: UnaryOperator::Neg,
                operand: Box::new(Expr::Number(4)),
            }),
            right: Box::new(Expr::Number(5)),
        };
        assert_eq!(evaluate(&tree), Ok(Rational::from_integer(-20)));
    }

    #[test]
    fn evaluate_text_rejects_a_zero_divisor() {
        for input in ["1/0", "1/(2-2)", "0/0"] {
            assert_eq!(
                evaluate_text(input),
                Err(ExpressionError::Eval(EvalError::DivisionByZero)),
                "input {input:?}"
            );
        }
    }

    #[test]
    fn evaluate_text_hands_back_a_parse_failure_unchanged() {
        assert_eq!(
            evaluate_text("1+"),
            Err(ExpressionError::Parse(ParseError {
                offset: 2,
                kind: ParseErrorKind::UnexpectedEndOfInput,
            }))
        );
    }

    #[test]
    fn evaluate_reports_overflow_from_either_operator_shape() {
        let doubled = format!("{max}+{max}", max = i64::MAX);
        assert_eq!(
            evaluate_text(&doubled),
            Err(ExpressionError::Eval(EvalError::Overflow))
        );
        let negated = Expr::UnaryOp {
            op: UnaryOperator::Neg,
            operand: Box::new(Expr::Number(i64::MIN)),
        };
        assert_eq!(evaluate(&negated), Err(EvalError::Overflow));
    }

    #[test]
    fn construction_moves_the_sign_and_reduces() {
        let cases: &[(i64, i64, i64, i64)] = &[
            (7, 1, 7, 1),
            (2, 6, 1, 3),
            (-2, 6, -1, 3),
            (2, -6, -1, 3),
            (-2, -6, 1, 3),
            (-2, 4, -1, 2),
            (5, 5, 1, 1),
            (0, 5, 0, 1),
            (i64::MIN, 1, i64::MIN, 1),
        ];
        for &(numerator, denominator, expected_numerator, expected_denominator) in cases {
            let value = ratio(numerator, denominator);
            assert_eq!(
                (value.numerator(), value.denominator()),
                (expected_numerator, expected_denominator),
                "input {numerator}/{denominator}"
            );
        }
    }

    #[test]
    fn arithmetic_stays_exact() {
        assert_eq!(ratio(1, 3).checked_add(ratio(1, 6)), Ok(ratio(1, 2)));
        assert_eq!(ratio(1, 2).checked_sub(ratio(1, 3)), Ok(ratio(1, 6)));
        assert_eq!(ratio(2, 3).checked_mul(ratio(3, 4)), Ok(ratio(1, 2)));
        assert_eq!(ratio(1, 2).checked_div(ratio(1, 4)), Ok(ratio(2, 1)));
        assert_eq!(ratio(1, 3).checked_neg(), Ok(ratio(-1, 3)));
    }

    #[test]
    fn a_zero_divisor_is_a_typed_error() {
        let zero = Rational::from_integer(0);
        assert_eq!(
            ratio(1, 2).checked_div(zero),
            Err(EvalError::DivisionByZero)
        );
        assert_eq!(Rational::new(1, 0), Err(EvalError::DivisionByZero));
    }

    #[test]
    fn every_checked_step_reports_overflow() {
        let biggest = Rational::from_integer(i64::MAX);
        let smallest = Rational::from_integer(i64::MIN);
        let tiny = ratio(1, i64::MAX);
        let opposite = ratio(-1, i64::MAX);
        let cases: &[(&str, Result<Rational, EvalError>)] = &[
            (
                "construction negates the numerator",
                Rational::new(i64::MIN, -1),
            ),
            (
                "construction negates the denominator",
                Rational::new(1, i64::MIN),
            ),
            (
                "a sum crosses the left numerator",
                biggest.checked_add(tiny),
            ),
            (
                "a sum crosses the right numerator",
                tiny.checked_add(biggest),
            ),
            ("a sum adds the crossed pair", biggest.checked_add(biggest)),
            (
                "a sum multiplies the denominators",
                tiny.checked_add(opposite),
            ),
            (
                "a difference crosses the left numerator",
                biggest.checked_sub(tiny),
            ),
            (
                "a difference crosses the right numerator",
                tiny.checked_sub(biggest),
            ),
            (
                "a difference subtracts the crossed pair",
                biggest.checked_sub(ratio(-1, 1)),
            ),
            (
                "a difference multiplies the denominators",
                tiny.checked_sub(tiny),
            ),
            (
                "a product multiplies the numerators",
                biggest.checked_mul(biggest),
            ),
            (
                "a product multiplies the denominators",
                tiny.checked_mul(tiny),
            ),
            (
                "a quotient crosses the numerator",
                biggest.checked_div(tiny),
            ),
            (
                "a quotient crosses the denominator",
                tiny.checked_div(biggest),
            ),
            ("a negation flips the numerator", smallest.checked_neg()),
        ];
        for &(name, outcome) in cases {
            assert_eq!(outcome, Err(EvalError::Overflow), "case {name:?}");
        }
    }
}
