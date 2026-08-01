// SPDX-License-Identifier: Apache-2.0
// Copyright Authors of Proofhouse

//! Property checks over the whole expression pipeline.
//!
//! The tests beside each source file pin a table of concrete inputs. This
//! suite widens the input to every shape the generators reach and holds the
//! pipeline to the laws that survive that widening: a rendering the parser
//! recovers, a scan that answers for any text at all, a reduction whose
//! result is already in lowest terms, and two walks of one tree agreeing.

// The lint wants every #[test] inside a #[cfg(test)] module, which is what
// keeps test code out of a library build. Cargo builds this file only as a
// test binary, so the whole file is already that module.
#![expect(
    clippy::tests_outside_test_module,
    reason = "an integration test file compiles as a test target and nothing else"
)]

use proofhouse_rust_lib::errors::ExpressionError;
use proofhouse_rust_lib::evaluator::concurrent::evaluate_parallel;
use proofhouse_rust_lib::evaluator::{Rational, evaluate, evaluate_text};
use proofhouse_rust_lib::formatter::format_expr;
use proofhouse_rust_lib::lexer::tokenize;
use proofhouse_rust_lib::parser::parse;
use proofhouse_rust_lib::testing::{expression_texts, expressions, spelling, token_sequences};
use proptest::{prop_assert, prop_assert_eq, proptest};

/// Nesting depth the tree strategies stop at.
///
/// Deep enough for every node type to turn up under both operands of an
/// infix operator, which is where the parenthesizing rules and the split
/// across threads earn their keep. A deeper tree repeats those shapes at a
/// price the parallel walk pays in threads.
const DEPTH: u32 = 4;

/// Longest token run drawn.
///
/// Long enough for a number to land next to another number and for a
/// bracket to land next to an operator, which is where a scan running two
/// tokens together would show.
const RUN_LENGTH: usize = 8;

/// Pattern the arbitrary-text property draws from: any string at all.
///
/// The `s` flag lets the wildcard reach a line break, so the draw covers
/// control characters and stray symbols along with the ordinary ones. None
/// of them can begin a token, and the point of the property is that the
/// scan says so rather than walking off an end.
const ANY_TEXT: &str = "(?s).*";

proptest! {
    /// A canonical rendering parses back to the tree it came from.
    ///
    /// The formatter drops every parenthesis it can, so the law it answers
    /// to is that the reader still recovers the tree the writer had. A
    /// dropped parenthesis that changed the grouping fails here on a shape
    /// no hand-written table would think to try.
    #[test]
    fn a_rendering_parses_back_to_its_own_tree(expr in expressions(DEPTH)) {
        prop_assert_eq!(parse(&format_expr(&expr)), Ok(expr));
    }

    /// The scan answers for any text, and the offsets it reports point at
    /// what they name.
    ///
    /// Handed a string from anywhere, the lexer has two outcomes and no
    /// third: a run of tokens, or one failure naming the character that
    /// stopped it. Every offset either outcome carries has to index the
    /// text it came from, and reach the one character it speaks for, which
    /// is where a miscounted multi-byte character would surface.
    #[test]
    fn scanning_any_text_reports_offsets_that_land_on_it(text in ANY_TEXT) {
        match tokenize(&text) {
            Ok(run) => {
                for token in &run {
                    prop_assert!(
                        text.get(token.offset..)
                            .is_some_and(|rest| rest.starts_with(token.lexeme.as_str())),
                        "token {token:?} sits elsewhere in {text:?}"
                    );
                }
            }
            Err(error) => {
                prop_assert_eq!(
                    text.get(error.offset..).and_then(|rest| rest.chars().next()),
                    Some(error.character)
                );
            }
        }
    }

    /// Reducing well-formed text yields a value in lowest terms or a
    /// failure the walk itself raised.
    ///
    /// A drawn string parses by construction, so the scan and the grammar
    /// have nothing left to object to and any failure has to come from the
    /// walk. What the walk returns carries the one shape the value type
    /// promises: reducing a result a second time leaves it alone.
    #[test]
    fn reducing_drawn_text_yields_lowest_terms_or_a_walk_failure(
        text in expression_texts(DEPTH),
    ) {
        let outcome = evaluate_text(&text);
        prop_assert!(
            !matches!(
                outcome,
                Err(ExpressionError::Lex(_) | ExpressionError::Parse(_))
            ),
            "canonical text {text:?} failed before the walk"
        );
        if let Ok(value) = outcome {
            prop_assert!(value.denominator() > 0);
            prop_assert_eq!(Rational::new(value.numerator(), value.denominator()), Ok(value));
        }
    }

    /// Splitting a tree across threads gives what walking it in order
    /// gives.
    ///
    /// Both walks read the same operators over the same operands, so the
    /// only way their answers can part is through the table the workers
    /// share. Failures count as answers here: a tree the ordinary walk
    /// turns away has to meet the same refusal from the other.
    #[test]
    fn both_walks_of_a_tree_reach_one_answer(expr in expressions(DEPTH)) {
        prop_assert_eq!(evaluate_parallel(&expr), evaluate(&expr));
    }

    /// A drawn token run scans back to itself out of its own spelling.
    ///
    /// The generator hands out tokens the lexer could have produced, at the
    /// offsets the spelling puts them at. Scanning that spelling has to
    /// return the run unchanged, which fails the moment the generator pairs
    /// a kind with text the lexer reads differently or counts an offset
    /// wrong.
    #[test]
    fn a_drawn_token_run_scans_back_to_itself(run in token_sequences(RUN_LENGTH)) {
        prop_assert_eq!(tokenize(&spelling(&run)), Ok(run));
    }
}
