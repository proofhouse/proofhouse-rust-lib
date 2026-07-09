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

# Check that release builds are reproducible. Copy the working tree
# (minus .git and target, so the untracked Cargo.lock still rides
# along) into a temp dir, build it, record the rlib's sha256, run
# `cargo clean`, build a second time, and compare — failing on any
# mismatch.
#
# This is a same-dir double build, not the build-from-two-paths check
# the binary sibling can use. An rlib carries its crate metadata, and
# that metadata records source paths which --remap-path-scope=object
# leaves alone: the scope confines path rewriting to emitted objects,
# which is what keeps a final binary path-independent but also means
# two builds of this library from different directories embed different
# paths and never match, byte-identical source or not — the workspace
# path-hashing caveat, cargo#13586. Holding the directory fixed is the
# way to isolate genuine nondeterminism (timestamps, map iteration,
# codegen ordering) from that path noise until the caveat is resolved.
#
# The remap flags still run so object code stays path-independent for
# parity with the binary crate; --remap-path-scope=object is stable
# since 1.95, and cargo's trim-paths profile key would subsume these
# flags once it leaves nightly (cargo#12137). The library embeds no
# git-derived data, so excluding .git only trims the copy.
# SOURCE_DATE_EPOCH is exported for parity with the sibling repos;
# rustc ignores it on Linux and macOS today, so it only guards against
# future timestamp stamping.
[script]
build-repro-check:
    work=$(mktemp -d)
    trap 'rm -rf "$work"' EXIT
    rsync -a --exclude=.git --exclude=target "$PWD"/ "$work"/
    cd "$work"
    build() {
        RUSTFLAGS="--remap-path-prefix=$PWD=/build --remap-path-prefix=${CARGO_HOME:-$HOME/.cargo}=/cargo --remap-path-scope=object" \
        CARGO_INCREMENTAL=0 SOURCE_DATE_EPOCH={{ source_date_epoch }} \
        cargo build --release --locked
    }
    build
    sum_a=$(shasum -a 256 < target/release/libproofhouse_rust_lib.rlib)
    cargo clean
    build
    sum_b=$(shasum -a 256 < target/release/libproofhouse_rust_lib.rlib)
    if [[ "${sum_a%% *}" != "${sum_b%% *}" ]]; then
        echo "build not reproducible: rlib differs between runs" >&2
        exit 1
    fi
    echo "reproducible: ${sum_a%% *}"

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
