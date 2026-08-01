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
# The Go twin prepends GOPATH/bin the same way. home_directory() rather
# than env("HOME") because the file has to parse on hosts that set no
# HOME.
export PATH := env("CARGO_HOME", home_directory() + "/.cargo") + "/bin:" + env("PATH")

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

# Refresh non-brew tooling: Vale's synced style packages, plus the
# cargo-installed checkers Homebrew carries no formula for. Those float
# to whatever the registry serves, the way a brew formula tracks
# upstream on a working machine. The pull request gate installs a
# written-down version instead, so a merge decision always names the
# release behind it.
install-tools:
    vale sync
    cargo install --locked cargo-machete
    cargo install --locked cargo-modules

# Install the layer-contract checker and the toolchain it runs under.
# Deliberately left out of `setup`: a whole second toolchain plus the
# compiler internals a plugin links against is a long download to hand
# every contributor, and one gate is all that asks for it. The pull
# request gate provisions the same pair for itself, so skipping this
# costs nothing but a local run of `lint-pup`.
install-pup:
    rustup toolchain install nightly-2026-01-22 --profile minimal --component rust-src --component rustc-dev --component llvm-tools-preview
    cargo +nightly-2026-01-22 install cargo_pup --locked

# --- Build ---

# Build the library in release mode
build:
    cargo build --release

# Remove the build trees. The layer checker keeps a second one beside
# target/ because it hands cargo a directory of its own, and a stale
# copy of that one outlives a plain `cargo clean`.
clean:
    cargo clean
    rm -rf .pup

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

# Run the test suite under nextest, which starts a process per test.
# One test taking its process down is then that test failing, not the
# end of the binary's run, and what comes back names every failure
# rather than the earliest one. Both workspace members are in scope.
# Profiles live in .config/nextest.toml and the merge path names the
# stricter of them on the command line. Trailing arguments pass through,
# so `just test <name>` filters.
test *args:
    cargo nextest run --workspace "$@"

# Run the doctests. The runner above reaches none of them: rustdoc
# exposes no stable entry point another harness can drive, and the
# request to build one has sat open since 2022. Documentation examples
# depend on this recipe and a merge-path step of its own to run at all.
test-doc:
    cargo test --doc

# Replay the concurrency models under loom, which reruns each model
# once per ordering the memory model allows its threads and atomics and
# stops at the first replay that breaks an assertion. The cfg flag is
# what swaps the standard library's primitives for loom's recording
# ones, so the models compile to nothing without it and an ordinary
# `just test` walks past the file.
#
# What that search reads is publication order: whether a thread can see
# the ready flag of a cell ahead of the writes the flag announces, and
# whether two threads racing for one key can run its computation twice.
# A test suite cannot ask either question, since both depend on an
# ordering a single run happens not to take.
#
# Each switch point multiplies the orderings left to try, so the bound
# on preemptions is what keeps the run in seconds: three forced
# switches per model, over models of two threads and a handful of
# atomic accesses each. Left unbounded the same models take minutes and
# find nothing further. That cost is why this stays a recipe the pull
# request gate calls rather than a hook every push waits on.
test-loom:
    RUSTFLAGS="--cfg loom" LOOM_MAX_PREEMPTIONS=3 cargo nextest run -p proofhouse-rust-lib --test loom

# --- Coverage ---

# Toolchain the coverage run compiles under. Branch instrumentation is
# still an unstable feature, so it answers on a nightly compiler only,
# and a dated channel keeps two runs of one commit on the same one.
# Nothing bumps this date on its own: raise it by hand, rerun `just
# cover`, and move the workflow along with it — the cover job in
# .github/workflows/ci.yml sets up the same channel and points back here.
coverage_toolchain := "nightly-2026-08-01"

# Run the suite under branch instrumentation and hold the result at every
# line and every branch.
#
# --remap-path-prefix rewrites the workspace root out of the tracefile,
# so a path one operating system writes matches what the others write and
# the merge downstream lands one entry per source file. --skip-functions
# drops the per-function records; nothing reads them here and they are
# where a merge across platforms finds contradictions. The task runner
# leaves the measured surface through --exclude: it is the thing reading
# the report, and asking a report to speak for its own reader is
# circular. The region threshold on the run is the backstop the coverage
# tool enforces by itself; `cargo xtask coverage` then reads the
# tracefile and answers for lines and branches, which is the pair this
# project gates on.
#
# The slot argument names the tracefile. CI hands it the runner label so
# every matrix slot uploads a file of its own; locally it stays "local".
cover slot="local":
    cargo +{{ coverage_toolchain }} llvm-cov nextest --branch --all-features --workspace --exclude xtask --remap-path-prefix --skip-functions --fail-under-regions 100 --lcov --output-path lcov-{{ slot }}.info
    cargo xtask coverage lcov-{{ slot }}.info

# Render the per-line HTML report and name the entry point. The source
# view shades each line and each branch arm by whether a test reached it,
# which points at the arm a new test still has to exercise. Local
# inspection only, so it names no threshold.
cover-html:
    cargo +{{ coverage_toolchain }} llvm-cov nextest --branch --all-features --workspace --exclude xtask --html
    @echo "open target/llvm-cov/html/index.html"

# Merge every slot's tracefile into one and check the merged total. This
# is the authoritative reading: a branch that no single operating system
# exercises still has to be reached by some slot, and only the merged
# file says whether it was. lcov does the merging rather than the task
# runner, because merging means combining the per-line hit counts of two
# reports over one file, where adding the summary lines of both would
# ask every slot to carry the whole tree alone. The CI coverage job
# runs this after collecting the per-slot artifacts.
#
# Two adjustments make that merge mean what it says. A Windows runner
# writes its source paths with a backslash, so the same file arrives
# under a second spelling and the merge would keep the two apart;
# rewriting the separator first lands one entry per source file.
# Branch records are the other: lcov discards them unless the run asks
# for them, and a merged report with no branch records left would clear
# a branch threshold by having nothing to fail on.
[script]
cover-combine:
    tracefiles=()
    for slot in lcov-*.info; do
        normalized=$(mktemp)
        tr '\\' '/' < "$slot" > "$normalized"
        mv "$normalized" "$slot"
        tracefiles+=(--add-tracefile "$slot")
    done
    lcov --branch-coverage "${tracefiles[@]}" --output-file lcov.info
    cargo xtask coverage lcov.info

# Fail when any line changed since [base] lacks coverage. The whole-tree
# floor already sits at every line, so on a clean branch this says
# nothing new; it earns its keep by naming the touched lines directly the
# moment a change does drop one, which reads faster than a total that
# moved. Reads the merged tracefile, so `cover-combine` runs first.
cover-diff base="origin/main":
    diff-cover lcov.info --compare-branch={{ base }} --fail-under=100

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

# Aggregator over the Rust-flavored lint sub-recipes: the rustfmt drift
# check, clippy across every lint group, the documentation render, the
# manifest scan for unused dependencies, the clone detector, the
# licensing audit, and actionlint over the workflow files. Carved out
# so a contributor working on the crate can rerun the compiler-adjacent
# gates without paying for the whole text-quality toolchain; each new
# Rust gate appends itself here, except the nightly sweep in
# `lint-udeps`, which no merge path may reach. actionlint rides along
# even though it reads YAML rather than Rust — it belongs to the same
# per-pull-request gate set, and the sibling repos give it the same
# slot.
lint-rs-all: lint-rs-format lint-clippy lint-docs lint-machete lint-dup-code lint-reuse lint-workflows

# Aggregate lint gate. Every checker the repo gains hangs off this one
# recipe, so there is a single command to run them all. The Rust gates
# arrive through `lint-rs-all`; prose, spelling, Markdown, JSON, YAML,
# TOML, this file's own layout, and the whitespace baseline follow.
lint: lint-rs-all lint-prose lint-spelling lint-markdown lint-config lint-yaml lint-toml lint-just lint-editorconfig

# Fail when rustfmt would rewrite any Rust source in the workspace.
# Reports the drift and leaves the tree alone; `format-rs` is the half
# that applies it, the same read-only and in-place pairing `lint-toml`
# and `format-toml` use. Settings come from rustfmt.toml, and --all
# reaches the xtask member alongside the library crate.
lint-rs-format:
    cargo fmt --all --check

# Run clippy over every workspace member, every target, and every
# feature. The lint selection lives in Cargo.toml's workspace lints
# tables and the thresholds in clippy.toml; the groups are configured as
# warnings there so an interactive run prints the whole list, and this
# recipe supplies the -D that turns any one of them into a failure. Tests
# and examples come along through --all-targets, which is where the
# gate would otherwise go blind.
lint-clippy:
    cargo clippy --workspace --all-targets --all-features -- -D warnings

# Render the workspace documentation and let the render itself be the
# gate. Nothing about these pages is published — the manifest turns
# publishing off, so no docs.rs build stands behind them and a broken
# link or an unparsable example would sit unread until someone opened
# the crate. Lint levels come from the rustdoc table in Cargo.toml;
# RUSTDOCFLAGS catches whatever warns outside it. --no-deps keeps the
# run on this workspace instead of re-rendering the dependency graph.
lint-docs:
    RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps

# Report dependencies the manifests declare and no source file names.
# machete searches the sources for each crate's identifier instead of
# building anything, which is what keeps it in a gate that runs on every
# pull request; the price is the odd miss on a crate reached through a
# macro. Proving the negative takes a compiler, and a run that slow
# belongs on a schedule of its own. --with-metadata would buy some of
# that accuracy by resolving the graph through cargo, at the risk of
# rewriting Cargo.lock from under a read-only gate, so it stays off.
lint-machete:
    cargo machete

# Name the dependencies the compiler never loads. Where the scan above
# reads text, this one builds every target across both workspace
# members and then asks rustc which of the resolved crates it read, so
# the cases a search has no way to reach — a crate named from inside a
# macro expansion, one a feature gate leaves out of the sources — get an
# answer at last. The flag carrying that answer has never stabilized,
# hence the nightly compiler. Nothing a merge waits on may rest on a
# toolchain that changes overnight, so this recipe stays outside
# lint-rs-all and a scheduled workflow calls it instead; lint-machete
# holds the pull request end of the same question.
lint-udeps:
    cargo +nightly udeps --workspace --all-targets

# Hunt for copy-pasted Rust. jscpd compares token streams rather than
# lines, so a clone stays visible after its bindings are renamed and
# its braces moved around. Settings live in .jscpd.json. Fifty tokens
# is the shortest run it will call a clone, which is upstream's own
# default written down so a change to it lands as a visible diff, and
# the zero percent ceiling turns one clone into a failed run — with a
# detector this coarse, anything it reports is worth a human deciding
# about. The scan is confined to Rust because that is the code under
# review here; pointed at the whole tree it also reads .vale.ini and
# the workflow files, whose repeated stanzas are how those formats are
# written rather than something to factor out. The flag drops the
# donation and product notices the tool prints once it is done.
lint-dup-code:
    jscpd --no-tips .

# Ask every tracked file who holds the copyright and under what terms.
# A Rust source answers with the two-line header it opens with, and
# reading that answer off the file rather than out of a list is the
# point of the gate. Everything else answers through REUSE.toml, where
# the same pair of facts is written once per group of files. The
# identifier those answers carry has to resolve to a license text, and
# LICENSES/ is the directory reuse resolves it against. Splitting the
# scan across processes is turned off: starting the pool takes longer
# than reading a tree this size, and a sandbox that withholds the
# semaphores behind it would leave no way to run the gate at all.
lint-reuse:
    reuse --no-multiprocessing lint

# Lint prose in Markdown files and source comments via vale. The glob
# drops both copies of the canonical Apache 2.0 text (the root LICENSE
# and the LICENSES/ directory reuse reads), the generated
# changelog, vale's own synced style packages, scratch dirs, the
# gitignored agent worktrees under .claude/worktrees/, the
# COMMIT_AGENTMSG draft (.vale.ini judges a commit message under its
# own stricter scope), the cargo build tree, and the second build tree
# the layer checker keeps beside it; the per-file-type
# rules in .vale.ini decide what else gets inspected. Findings render
# through the proofhouse-agent template from the proofhouse package:
# one machine-parseable line per finding.
lint-prose *args:
    vale --output=proofhouse-agent.tmpl --glob='!{LICENSE,LICENSES/*,CHANGELOG.md,.vale/*,tmp/*,.claude/worktrees/*,COMMIT_AGENTMSG,target/*,.pup/*}' {{ if args == "" { "." } else { args } }}

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
# skip it for that reason — and LICENSES/, whose contents are upstream
# license text nobody here may reflow. The tool's own default excludes
# drop Cargo.lock and the binary formats on top of that. Upstream's release
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
# customManager. It sits inside `lint-rs-all`, so the merge path reads
# these files here as well as through the shared lint-workflows
# workflow. A container runtime is the one dependency no other member
# of the set carries.
lint-workflows:
    {{ actionlint }}

# Refuse a source file that sits in the crate directory with no `mod`
# declaration reaching it. Such a file compiles nowhere and is read as
# live code by everyone who opens it. cargo-modules resolves the crate
# the way an editor does rather than by reading source text, so a
# module a macro declares counts as reached.
#
# Cycles are what this recipe says nothing about. The same subcommand
# set offers that check and 0.26.0 cannot run it here: the search
# covers the item graph before any filter narrows it, and a type owning
# a method that names the type is a cycle by that reading, so every
# crate with an impl block fails it. pup.ron carries cycle-freedom
# instead, by listing per layer which layers are open to it — every
# crate-internal import pointing down one order, no set of modules can
# close a loop — and `lint-pup` below is what reads it.
#
# Neither this recipe nor `lint-pup` joins `lint-rs-all`, both being
# far too expensive to install for the job every ordinary checker
# shares. The `arch` job in ci.yml runs the pair instead and blocks a
# merge on them all the same.
lint-arch:
    cargo modules orphans --deny --package proofhouse-rust-lib --lib

# Check the layer contract written down in pup.ron, which is the
# question the graph shape above leaves open: not whether one module
# reaches another, but whether it may. cargo-pup is a compiler plugin,
# so it reads imports the compiler already resolved and loads only
# under the nightly it was built against. That date is spelled out
# here and moves when a cargo-pup release moves it, with no dependency
# manager watching the pair, so treat it as a note to whoever upgrades
# the tool. The same nightly predates the minimum release this crate
# supports, hence the flag waving that floor through — the run
# type-checks the crate to inspect it and produces nothing anyone
# installs. Get the toolchain and the plugin with `install-pup`.
lint-pup:
    cargo +nightly-2026-01-22 pup check --ignore-rust-version

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

# Aggregate in-place formatter. Grows per gate; carries the Rust,
# Markdown, JSON, TOML, and Justfile fixers today.
format: format-rs format-markdown format-config format-toml format-just

# Rewrite Rust source in rustfmt's canonical shape, across every
# workspace member. The gate that reports the same drift without
# touching anything is `lint-rs-format`, which is what CI runs.
format-rs:
    cargo fmt --all

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
