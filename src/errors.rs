// SPDX-License-Identifier: Apache-2.0
// Copyright Authors of Proofhouse

//! Errors raised while processing expressions.

use std::error::Error;
use std::fmt;

/// A character the lexer cannot turn into a token, with where it was found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LexError {
    /// Byte offset of the offending character within the source text.
    pub offset: usize,
    /// The character that could not begin a token.
    pub character: char,
}

impl fmt::Display for LexError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "unexpected character {:?} at offset {}",
            self.character, self.offset
        )
    }
}

impl Error for LexError {}

/// Any error raised while turning expression text into a result.
///
/// Only lexing can fail today; parsing and evaluation add their own
/// variants as those stages land, so a caller has one type to match on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExpressionError {
    /// The lexer met a character it could not tokenize.
    Lex(LexError),
}

impl fmt::Display for ExpressionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Lex(err) => err.fmt(f),
        }
    }
}

impl Error for ExpressionError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Lex(err) => Some(err),
        }
    }
}

impl From<LexError> for ExpressionError {
    fn from(err: LexError) -> Self {
        Self::Lex(err)
    }
}

#[cfg(test)]
mod tests {
    use super::{ExpressionError, LexError};
    use std::error::Error;

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
        let source = wrapped.source().and_then(|e| e.downcast_ref::<LexError>());
        assert_eq!(source, Some(&inner));
    }
}
