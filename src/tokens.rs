// SPDX-License-Identifier: Apache-2.0
// Copyright Authors of Proofhouse

//! Token kinds and the token record the lexer produces.

/// Classification of a lexed token.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenKind {
    /// A run of decimal digits forming an integer literal.
    Number,
    /// The `+` operator.
    Plus,
    /// The `-` operator.
    Minus,
    /// The `*` operator.
    Star,
    /// The `/` operator.
    Slash,
    /// A left parenthesis `(`.
    LParen,
    /// A right parenthesis `)`.
    RParen,
}

/// One token produced by the lexer, pairing matched text with its kind.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Token {
    /// What kind of token this is.
    pub kind: TokenKind,
    /// The exact source text the token matched.
    pub lexeme: String,
    /// Byte offset of the token's first character within the source text.
    pub offset: usize,
}
