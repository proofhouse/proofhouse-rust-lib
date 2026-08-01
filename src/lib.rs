// SPDX-License-Identifier: Apache-2.0
// Copyright Authors of Proofhouse

//! Reference library for the Proofhouse Rust lib reference repository.

pub mod ast;
pub mod cache;
pub mod errors;
pub mod evaluator;
pub mod formatter;
pub mod lexer;
pub mod parser;
pub mod sync;
mod sync_shim;
// Reaches a build only when someone asks for the generators, which is
// what keeps the search library off the graph of an ordinary one.
#[cfg(feature = "testing")]
pub mod testing;
pub mod tokens;

/// Returns the crate version, resolved at compile time from the package
/// manifest so it always tracks the version `Cargo.toml` declares.
#[must_use]
pub const fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

#[cfg(test)]
mod tests {
    use super::version;

    /// Whether `part` reads as one component of a version: at least one
    /// character, every one of them a digit.
    ///
    /// The condition sits in a function of its own rather than inside
    /// the assertion that reads it. Whatever version this crate declares
    /// satisfies both halves, so an `&&` written inline would leave the
    /// failing side of each half with nothing to drive it. The case
    /// table that follows supplies what a real version withholds.
    fn is_numeric_component(part: &str) -> bool {
        !part.is_empty() && part.chars().all(|ch| ch.is_ascii_digit())
    }

    #[test]
    fn version_is_three_numeric_parts() {
        let parts: Vec<&str> = version().split('.').collect();
        assert_eq!(parts.len(), 3, "version is not a three-part semver");
        for part in parts {
            assert!(
                is_numeric_component(part),
                "version component `{part}` is not all digits"
            );
        }
    }

    #[test]
    fn a_component_is_digits_and_at_least_one_of_them() {
        let cases = [("0", true), ("142", true), ("", false), ("1a", false)];
        for (part, expected) in cases {
            assert_eq!(
                is_numeric_component(part),
                expected,
                "component `{part}` read the wrong way"
            );
        }
    }
}
