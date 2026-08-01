set unstable
set positional-arguments

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

# Locate a Docker-compatible container runtime. Probe PATH first, then
# well-known install locations so the recipe still works inside agentic
# harnesses or sandboxes that strip /usr/local/bin from PATH. Override by
# setting CONTAINER_RUNTIME in the environment.
#
# The continuation lines of the `for` list below hang under the first
# candidate path rather than on a two-space grid, which is what shell
# style calls for and what `lint-editorconfig` would otherwise reject
# under this file's indent_size = 2. Exempt just that span rather than
# re-indent a block the sibling repos carry verbatim.
# editorconfig-checker-disable
container_runtime := env("CONTAINER_RUNTIME", `bash -c '
    docker_path=$(command -v docker 2>/dev/null || true)
    podman_path=$(command -v podman 2>/dev/null || true)
    for p in "$docker_path" \
             /usr/local/bin/docker \
             /opt/homebrew/bin/docker \
             /Applications/Docker.app/Contents/Resources/bin/docker \
             "$HOME/.orbstack/bin/docker" \
             "$HOME/.rd/bin/docker" \
             "$podman_path" \
             /opt/podman/bin/podman; do
        if [ -n "$p" ] && [ -x "$p" ]; then echo "$p"; exit 0; fi
    done
    echo docker
'`)

# editorconfig-checker-enable

# actionlint version pin. The upstream image bundles actionlint (and the
# shellcheck it shells out to) at a known version, so we pin the image
# by digest rather than install either tool on the host. Renovate
# tracks the version + digest pair via the Justfile customManager in
# the shared org preset (see .github/renovate.json5).
#
# renovate: datasource=docker depName=rhysd/actionlint
actionlint_version := "1.7.12"
actionlint_image := "docker.io/rhysd/actionlint:1.7.12@sha256:b1934ee5f1c509618f2508e6eb47ee0d3520686341fec936f3b79331f9315667"

# actionlint invocation. Mounts the repo read-only at /repo with -w /repo
# so actionlint finds .github/workflows/ and .github/actionlint.yaml.
# DOCKER_CONFIG points at a fresh empty directory so docker skips the
# osxkeychain credential helper (public Docker Hub pulls don't need it,
# and sandboxed environments can't always reach the helper binary);
# PATH gets the runtime's directory prepended for cases where docker
# itself isn't on the calling shell's PATH.
actionlint := 'DOCKER_CONFIG="$(mktemp -d)" PATH="$(dirname ' + container_runtime + '):$PATH" ' + container_runtime + ' run --rm -v "$(pwd):/repo:ro" -w /repo ' + actionlint_image

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

# --- Setup ---

# Set up development environment. New contributors run this once after
# cloning. Idempotent: re-running upgrades dependencies and refreshes
# Vale's synced style packages.
setup:
    just install-brew
    just install-tools

# Install Homebrew dependencies from Brewfile.
install-brew:
    brew bundle check || brew bundle install

# Refresh non-brew tooling. Today that means Vale's synced style
# packages; grows as new sync-style tools land.
install-tools:
    vale sync

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
# Markdown, JSON, YAML, TOML, this file's own layout, and the
# whitespace baseline are its members today.
lint: lint-prose lint-spelling lint-markdown lint-config lint-yaml lint-toml lint-just lint-editorconfig

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

# Check that just's own formatter would change nothing in this Justfile.
# Read-only twin of `format-just`; run that to apply the rewrite. The
# build tool that every other gate is invoked through was itself the one
# file no gate read, so this closes that loop. See `format-just` for why
# --unstable is spelled out on the command line.
lint-just:
    just --fmt --check --unstable

# Check every tracked file against the rules in .editorconfig — charset,
# line endings, final newline, trailing whitespace, indent style and
# size. The other gates each own one language; this one holds the
# baseline that spans all of them, including the file types no linter in
# the set reads (.gitattributes, .gitignore, the Brewfile, CODEOWNERS).
# Scope and behavior come from .editorconfig-checker.json, whose Exclude
# list mirrors the top-level `exclude:` in .pre-commit-config.yaml so the
# hook and the recipe judge the same files, plus CHANGELOG.md, which
# `cog changelog` regenerates wholesale and leaves without a final
# newline every release — the vale hook and the prose recipes already
# skip it for that reason. The tool's own default excludes drop
# Cargo.lock and the binary formats on top of that. Upstream's release
# archives also carry a short `ec` alias, but the Homebrew formula this
# repo provisions from builds the long name only, so the recipe spells
# it out.
lint-editorconfig:
    editorconfig-checker

# Lint GitHub Actions workflow files via actionlint. actionlint walks
# `.github/workflows/` by default, parses each workflow, and flags
# unknown actions, mis-typed expressions, shellcheck issues inside
# `run:` blocks, and SHA-pin drift. Complements `lint-yaml` (which
# checks YAML structure) with workflow-shape rules yamllint can't see.
# Runs from the digest-pinned Docker image declared at the top of this
# file; Renovate bumps the version + digest via the shared Justfile
# customManager. Deliberately outside the `lint` aggregate: no other
# member needs a container runtime, and the shared lint-workflows
# workflow already gates these files on every pull request.
lint-workflows:
    {{ actionlint }}

# Pre-validate a drafted commit message against the same gates the
# commit-msg hook runs, so message problems surface while iterating
# rather than at commit time. Reads the draft from the repo-root
# COMMIT_AGENTMSG file (gitignored; see AGENTS.md for the workflow) and
# runs the commit-msg stage through prek, which fires the four shared
# hooks from proofhouse/pre-commit-hooks: commit-trailers, commitlint,
# vale-commit-msg, and cspell-commit-msg. The real gate stays the prek
# commit-msg hook on .git/COMMIT_EDITMSG; this recipe only mirrors it.
# Commit the validated draft with `git commit -F COMMIT_AGENTMSG`.
lint-commit-msg:
    prek run --stage commit-msg --commit-msg-filename COMMIT_AGENTMSG

# --- Format ---

# Aggregate in-place formatter. Grows per gate; carries the Markdown, JSON,
# TOML, and Justfile fixers today.
format: format-markdown format-config format-toml format-just

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

# Rewrite this Justfile in just's canonical formatting. Read-only twin
# is `lint-just`. --unstable is required because `--fmt` is still gated
# behind just's unstable flag; the `set unstable` at the top of the file
# governs recipe attributes, not command-line subcommands, so the flag
# has to be passed here as well. Adopting the gate cost a single
# rewrite: the two boolean settings opening the file gave up their
# `:= true` tails for the bare shorthand the formatter emits.
format-just:
    just --fmt --unstable

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

# Run pre-commit hooks on changed files (the everyday invocation).
prek:
    prek

# Run pre-commit hooks on every file in the tree. Useful after a
# hook config change or before a release sweep.
prek-all:
    prek run --all-files

# Install the project's pre-commit hooks (commit-msg, pre-commit,
# pre-push). New contributors run this once after `just setup`; the
# `just setup` recipe does NOT run it automatically because installing
# hooks modifies .git/ and contributors may prefer to opt in.
prek-install:
    prek install -t commit-msg -t pre-commit -t pre-push

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
