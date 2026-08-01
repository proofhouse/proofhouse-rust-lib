// SPDX-License-Identifier: Apache-2.0
// Copyright Authors of Proofhouse

//! Errors raised while processing expressions.

use std::error::Error;
use std::fmt;

/// A character the lexer can't turn into a token, and its position.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LexError {
    /// Byte offset of the offending character within the source text.
    pub offset: usize,
    /// The character that couldn't begin a token.
    pub character: char,
}

impl fmt::Display for LexError {
    #[expect(
        clippy::use_debug,
        reason = "the quoted Debug rendering is what keeps a stray control character legible"
    )]
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "unexpected character {:?} at offset {}",
            self.character, self.offset
        )
    }
}

impl Error for LexError {}

/// What the parser found wrong with a token stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParseErrorKind {
    /// A token turned up where the grammar has no use for one.
    UnexpectedToken,
    /// The input ran out with a production still unfinished.
    UnexpectedEndOfInput,
    /// More tokens follow an expression that was already complete.
    TrailingInput,
    /// A digit run names a value too large for the 64-bit integer that
    /// holds it.
    NumberOutOfRange,
}

impl fmt::Display for ParseErrorKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let phrase = match *self {
            Self::UnexpectedToken => "unexpected token",
            Self::UnexpectedEndOfInput => "unexpected end of input",
            Self::TrailingInput => "trailing input after a complete expression",
            Self::NumberOutOfRange => "number literal out of range",
        };
        f.write_str(phrase)
    }
}

/// A token stream the parser can't turn into an expression, and where it
/// gave up.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ParseError {
    /// Byte offset the failure points at within the source text. A
    /// failure that runs off the end of the input points one past its
    /// last byte.
    pub offset: usize,
    /// Which way the token stream disagreed with the grammar.
    pub kind: ParseErrorKind,
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} at offset {}", self.kind, self.offset)
    }
}

impl Error for ParseError {}

/// Any error raised while turning expression text into a result.
///
/// Lexing and parsing can fail today. Evaluation adds its own variant as
/// that stage lands, so a caller has one type to match on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExpressionError {
    /// The lexer met a character it couldn't tokenize.
    Lex(LexError),
    /// The parser met a token stream that forms no expression.
    Parse(ParseError),
}

impl fmt::Display for ExpressionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Lex(err) => err.fmt(f),
            Self::Parse(err) => err.fmt(f),
        }
    }
}

impl Error for ExpressionError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Lex(err) => Some(err),
            Self::Parse(err) => Some(err),
        }
    }
}

impl From<LexError> for ExpressionError {
    fn from(err: LexError) -> Self {
        Self::Lex(err)
    }
}

impl From<ParseError> for ExpressionError {
    fn from(err: ParseError) -> Self {
        Self::Parse(err)
    }
}

#[cfg(test)]
mod tests {
    use super::{ExpressionError, LexError, ParseError, ParseErrorKind};
    use std::error::Error as _;

    #[test]
    fn parse_error_display_names_kind_and_offset() {
        let cases: &[(ParseErrorKind, &str)] = &[
            (
                ParseErrorKind::UnexpectedToken,
                "unexpected token at offset 4",
            ),
            (
                ParseErrorKind::UnexpectedEndOfInput,
                "unexpected end of input at offset 4",
            ),
            (
                ParseErrorKind::TrailingInput,
                "trailing input after a complete expression at offset 4",
            ),
            (
                ParseErrorKind::NumberOutOfRange,
                "number literal out of range at offset 4",
            ),
        ];
        for &(kind, expected) in cases {
            let err = ParseError { offset: 4, kind };
            assert_eq!(err.to_string(), expected);
        }
    }

    #[test]
    fn expression_error_wraps_and_delegates_to_parse_error() {
        let inner = ParseError {
            offset: 2,
            kind: ParseErrorKind::UnexpectedToken,
        };
        let wrapped = ExpressionError::from(inner);
        assert_eq!(wrapped, ExpressionError::Parse(inner));
        assert_eq!(wrapped.to_string(), inner.to_string());
        let source = wrapped
            .source()
            .and_then(|err| err.downcast_ref::<ParseError>());
        assert_eq!(source, Some(&inner));
    }

    #[test]
    fn lex_error_display_names_character_and_offset() {
        let err = LexError {
            offset: 3,
            character: '$',
        };
        assert_eq!(err.to_string(), "unexpected character '$' at offset 3");
    }

    #[test]
    fn expression_error_wraps_and_delegates_to_lex_error() {
        let inner = LexError {
            offset: 1,
            character: '.',
        };
        let wrapped = ExpressionError::from(inner);
        assert_eq!(wrapped, ExpressionError::Lex(inner));
        assert_eq!(wrapped.to_string(), inner.to_string());
        let source = wrapped
            .source()
            .and_then(|err| err.downcast_ref::<LexError>());
        assert_eq!(source, Some(&inner));
    }
}
