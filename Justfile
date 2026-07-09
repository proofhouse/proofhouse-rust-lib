set unstable := true
set positional-arguments := true

# Run [script] recipes under bash rather than the default sh. On Linux
# sh is dash, which lacks [[ ]], <<<, and set -o pipefail — constructs
# [script] recipes are free to rely on. macOS sh is bash, so a dash
# incompatibility would stay hidden locally until CI runs on Linux.
set script-interpreter := ['bash', '-eu']

# Put rustup's shim directory ahead of any distro or Homebrew cargo on
# PATH. When the shadowing binary wins, rust-toolchain.toml is silently
# ignored and recipes run under whatever compiler that binary carries.
# The Go twin prepends GOPATH/bin the same way.
export PATH := env("CARGO_HOME", env("HOME") + "/.cargo") + "/bin:" + env("PATH")

# Build metadata derived from git, following the Go twin so the whole
# derivation reads from one block. `date` is the committer date (UTC,
# ISO-8601), not the build's wall clock, so two builds of one commit
# agree on the instant, and `source_date_epoch` carries that instant as
# a unix timestamp for any tool that honors SOURCE_DATE_EPOCH.
#
# `--abbrev=7` / `--short=7` pin the abbreviated hash length so two
# checkouts of the same commit render the same string; left to
# core.abbrev=auto the length tracks a repo's object count, which shifts
# across shallow clones, freshly packed repos, and aged working copies.

version := `git describe --tags --abbrev=7 2>/dev/null || git rev-parse --short=7 HEAD 2>/dev/null || echo "DEV"`
commit := `git rev-parse --short=7 HEAD 2>/dev/null || echo ""`
date := `TZ=UTC git log -1 --format=%cd --date=format-local:%Y-%m-%dT%H:%M:%SZ 2>/dev/null || echo "unknown"`
source_date_epoch := `git log -1 --format=%ct 2>/dev/null || echo "0"`

# Default recipe
default: test

# --- Build ---

# Build the library in release mode.
build:
    cargo build --release

# --- Test ---

# Run the test suite. Trailing arguments pass through to cargo test, so
# `just test <name>` filters and `just test -- --nocapture` reaches the
# harness.
test *args:
    cargo test "$@"

# Run the doctests. Kept out of `test` because the coverage-oriented
# test runner that lands later cannot execute doctests, so they need
# their own entry point to stay exercised.
test-doc:
    cargo test --doc

# --- Clean ---

# Remove the target/ build tree.
clean:
    cargo clean
