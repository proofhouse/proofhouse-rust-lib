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

# Build the library in release mode
build:
    cargo build --release

# Remove the target/ build tree
clean:
    cargo clean

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

# --- Dependencies ---

# Check that Cargo.lock is in sync with the manifests. Under the locked
# flag `cargo metadata` reads the dependency graph without refreshing
# the lock, so a missing or stale Cargo.lock makes it fail rather than
# silently re-resolve. The `cargo update` variants are the wrong tool:
# they report any upgrade the registry offers, not whether the committed
# lock still matches Cargo.toml. CI runs this on every PR; contributors
# regenerate the lock and commit the result.
lock-check:
    cargo metadata --locked --format-version 1 > /dev/null

# --- Lint ---

# Aggregate lint gate. Every checker the repo gains hangs off this one
# recipe, so there is a single command to run them all. Prose, spelling,
# Markdown, JSON, YAML, and TOML are its members today.
lint: lint-prose lint-spelling lint-markdown lint-config lint-yaml lint-toml

# Lint prose in Markdown files and source comments via vale. The glob
# drops the LICENSE (canonical Apache 2.0 text), the generated
# changelog, vale's own synced style packages, scratch dirs, the
# gitignored agent worktrees under .claude/worktrees/, the
# COMMIT_AGENTMSG draft (.vale.ini judges a commit message under its
# own stricter scope), and the cargo build tree; the per-file-type
# rules in .vale.ini decide what else gets inspected. Findings render
# through the proofhouse-agent template from the proofhouse package:
# one machine-parseable line per finding.
lint-prose *args:
    vale --output=proofhouse-agent.tmpl --glob='!{LICENSE,CHANGELOG.md,.vale/*,tmp/*,.claude/worktrees/*,COMMIT_AGENTMSG,target/*}' {{ if args == "" { "." } else { args } }}

# Check spelling tree-wide with cspell, against the project dictionary
# at .cspell-words.txt plus the bundled Rust and crate-name
# dictionaries — the pair covers ordinary Rust identifiers and the
# dependency names in Cargo.toml without either landing in the project
# list. Generated and vendored trees drop out through ignorePaths in
# .cspell.jsonc. The COMMIT_AGENTMSG draft stays out too, since a
# half-written message would otherwise fail every tree-wide run.
lint-spelling *args:
    cspell --config .cspell.jsonc --no-summary --no-progress --no-must-find-files --exclude COMMIT_AGENTMSG {{ if args == "" { "." } else { args } }}

# Lint Markdown files against the project's .rumdl.toml ruleset.
# rumdl handles structural lints (heading style, list marker style,
# code fence style); vale handles prose.
lint-markdown *args:
    rumdl check {{ if args == "" { "." } else { args } }}

# Lint JSON / JS / TS files via biome. Recommended ruleset, biome's
# own formatter; covers config files (biome.json, .cspell.jsonc) and
# any future scripts under .github/actions/.
lint-config *args:
    biome check --files-ignore-unknown=true {{ if args == "" { "." } else { args } }}

# Lint YAML files (config, workflows, action definitions). --strict
# treats warnings as errors so the gate matches CI behavior; per-rule
# tuning lives in .yamllint.yaml.
lint-yaml *args:
    yamllint --strict {{ if args == "" { "." } else { args } }}

# tombi is the org TOML gate (tombi 1.2.0): lint-checks Cargo.toml (validated offline
# against the embedded SchemaStore cargo.json), rust-toolchain.toml, .cargo/config.toml,
# and workspace member manifests. Format gate runs in --check --diff so unformatted TOML
# fails without rewrite. Cargo.lock is excluded from formatting via tombi.toml. --offline
# keeps CI hermetic; --error-on-warnings makes warnings hard failures. Scope lives in
# tombi.toml, so no path args are passed.
lint-toml:
    tombi format --check --diff
    tombi lint --offline --error-on-warnings

# --- Format ---

# Aggregate in-place formatter. Grows per gate; carries the Markdown, JSON, and
# TOML fixers today.
format: format-markdown format-config format-toml

# Format Markdown files (whitespace, list markers, code fence styles).
# Rewrites in place. Pair with `fix-markdown` for semantic lint fixes.
format-markdown *args:
    rumdl fmt {{ if args == "" { "." } else { args } }}

# Format JSON / JS / TS files in place via biome's formatter.
format-config *args:
    biome format --write {{ if args == "" { "." } else { args } }}

# In-place TOML formatter — fixer paired with lint-toml's --check gate. Whitespace/style
# only; key order preserved (reordering disabled in tombi.toml).
format-toml:
    tombi format

# --- Fix ---

# Apply rumdl's auto-fixable rules to Markdown files. Complement to
# `format-markdown` (which only rewrites whitespace and ordering, not
# semantic lints).
fix-markdown *args:
    rumdl check --fix {{ if args == "" { "." } else { args } }}

# --- Utilities ---

# Sync Vale styles and dictionaries. Run once after cloning the repo,
# and whenever .vale.ini's Packages list changes. CI runs this before
# `just lint-prose`.
vale-sync:
    vale sync

# Generate the full CHANGELOG.md from Conventional Commit history.
# `cog changelog` emits Markdown without an H1; the pipeline prepends
# one and runs rumdl with MD024 (duplicate headings) disabled so
# adjacent releases with the same section names don't fight the
# linter.
generate-changelog:
    cog changelog | { echo "# Changelog"; cat; } | rumdl check -d MD024 --fix --stdin > CHANGELOG.md

# Preview the changelog entries since the last tagged release. Useful
# during release prep to see what `cog changelog` will emit before
# committing the regeneration.
preview-changelog:
    cog changelog --at $(git describe --tags)..HEAD -t full_hash | rumdl check -d MD041 --fix --stdin

# Generate release notes for a specific version (or for HEAD if no
# version is given). Output goes to stdout; pipe to a file or paste
# into the GitHub release body.
[script]
generate-release-notes version="":
    v=$([[ -n "{{ version }}" ]] && echo "v{{ version }}" || echo "..$(git rev-parse HEAD)")
    cog changelog --at $v -t full_hash | rumdl check -d MD024,MD041 --isolated --fix --stdin
