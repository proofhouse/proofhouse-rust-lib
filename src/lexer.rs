// SPDX-License-Identifier: Apache-2.0
// Copyright Authors of Proofhouse

//! Lexer turning expression text into a token stream.

use crate::errors::LexError;
use crate::tokens::{Token, TokenKind};

/// Split expression text into tokens.
///
/// Whitespace separates tokens and means nothing on its own. A maximal
/// run of decimal digits becomes one [`TokenKind::Number`], and each of
/// the six operator and parenthesis characters maps to its own kind.
/// Empty or whitespace-only input yields an empty vector.
///
/// # Errors
///
/// Returns a [`LexError`] naming the byte offset and the character when the
/// input holds anything that can't begin a token.
#[expect(
    clippy::arithmetic_side_effects,
    reason = "the offsets and lengths added here index a string already in memory"
)]
#[expect(
    clippy::string_slice,
    reason = "both bounds come from char_indices, so they sit on character boundaries"
)]
pub fn tokenize(text: &str) -> Result<Vec<Token>, LexError> {
    let mut tokens = Vec::new();
    let mut chars = text.char_indices().peekable();
    while let Some((offset, ch)) = chars.next() {
        if ch.is_ascii_digit() {
            let mut end = offset + ch.len_utf8();
            while let Some((idx, digit)) = chars.next_if(|&(_, next)| next.is_ascii_digit()) {
                end = idx + digit.len_utf8();
            }
            tokens.push(Token {
                kind: TokenKind::Number,
                lexeme: text[offset..end].to_owned(),
                offset,
            });
        } else if let Some(kind) = single_char_kind(ch) {
            tokens.push(Token {
                kind,
                lexeme: ch.to_string(),
                offset,
            });
        } else if !ch.is_whitespace() {
            return Err(LexError {
                offset,
                character: ch,
            });
        }
    }
    Ok(tokens)
}

/// Map one of the six operator or parenthesis characters to its kind.
const fn single_char_kind(ch: char) -> Option<TokenKind> {
    match ch {
        '+' => Some(TokenKind::Plus),
        '-' => Some(TokenKind::Minus),
        '*' => Some(TokenKind::Star),
        '/' => Some(TokenKind::Slash),
        '(' => Some(TokenKind::LParen),
        ')' => Some(TokenKind::RParen),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::tokenize;
    use crate::errors::LexError;
    use crate::tokens::{Token, TokenKind};

    fn tok(kind: TokenKind, lexeme: &str, offset: usize) -> Token {
        Token {
            kind,
            lexeme: lexeme.to_owned(),
            offset,
        }
    }

    #[test]
    fn tokenize_yields_expected_tokens() {
        let cases: &[(&str, Vec<Token>)] = &[
            ("7", vec![tok(TokenKind::Number, "7", 0)]),
            ("1234", vec![tok(TokenKind::Number, "1234", 0)]),
            ("007", vec![tok(TokenKind::Number, "007", 0)]),
            (
                "1+2",
                vec![
                    tok(TokenKind::Number, "1", 0),
                    tok(TokenKind::Plus, "+", 1),
                    tok(TokenKind::Number, "2", 2),
                ],
            ),
            (
                " 12 * (34 - 5) / 6 ",
                vec![
                    tok(TokenKind::Number, "12", 1),
                    tok(TokenKind::Star, "*", 4),
                    tok(TokenKind::LParen, "(", 6),
                    tok(TokenKind::Number, "34", 7),
                    tok(TokenKind::Minus, "-", 10),
                    tok(TokenKind::Number, "5", 12),
                    tok(TokenKind::RParen, ")", 13),
                    tok(TokenKind::Slash, "/", 15),
                    tok(TokenKind::Number, "6", 17),
                ],
            ),
        ];
        for (input, expected) in cases {
            assert_eq!(tokenize(input).unwrap(), *expected, "input {input:?}");
        }
    }

    #[test]
    fn tokenize_maps_each_operator_and_paren() {
        let kinds: Vec<TokenKind> = tokenize("+-*/()")
            .unwrap()
            .iter()
            .map(|token| token.kind)
            .collect();
        assert_eq!(
            kinds,
            vec![
                TokenKind::Plus,
                TokenKind::Minus,
                TokenKind::Star,
                TokenKind::Slash,
                TokenKind::LParen,
                TokenKind::RParen,
            ]
        );
    }

    #[test]
    fn tokenize_empty_or_whitespace_yields_no_tokens() {
        for input in ["", " ", " \t\n  "] {
            assert_eq!(
                tokenize(input).unwrap(),
                Vec::<Token>::new(),
                "input {input:?}"
            );
        }
    }

    #[test]
    fn tokenize_rejects_stray_character_with_byte_offset() {
        let cases: &[(&str, usize, char)] = &[
            ("a", 0, 'a'),
            ("12 $ 3", 3, '$'),
            ("1.5", 1, '.'),
            // The two-byte no-break space counts as whitespace, so the
            // three-byte '€' lands at byte offset 2 while standing second
            // in the string. Offsets count bytes, never characters.
            ("\u{a0}\u{20ac}", 2, '\u{20ac}'),
        ];
        for &(input, offset, character) in cases {
            assert_eq!(
                tokenize(input),
                Err(LexError { offset, character }),
                "input {input:?}"
            );
        }
    }
}
