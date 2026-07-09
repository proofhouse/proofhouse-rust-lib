// SPDX-License-Identifier: Apache-2.0
// Copyright Authors of Proofhouse

//! Reference library for the Proofhouse Rust lib reference repository.

/// Returns the crate version, resolved at compile time from the package
/// manifest so it always tracks the version `Cargo.toml` declares.
#[must_use]
pub const fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

#[cfg(test)]
mod tests {
    use super::version;

    #[test]
    fn version_is_three_numeric_parts() {
        let parts: Vec<&str> = version().split('.').collect();
        assert_eq!(parts.len(), 3, "version is not a three-part semver");
        for part in parts {
            assert!(
                !part.is_empty() && part.chars().all(|c| c.is_ascii_digit()),
                "version component `{part}` is not all digits"
            );
        }
    }
}
