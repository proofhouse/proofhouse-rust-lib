// SPDX-License-Identifier: Apache-2.0
// Copyright Authors of Proofhouse

//! Workspace task runner for the Proofhouse Rust lib reference repository.
//!
//! One task lives here. `coverage` reads lcov tracefiles, adds up the line and
//! branch records they carry, prints what each source file reached, and fails
//! unless the tests reached every instrumented line and every instrumented
//! branch.
//!
//! The coverage tool holds a single run to its own region threshold, and it has
//! no answer for the set of them. Each matrix slot writes a tracefile of its
//! own, and only the merged file says what those runs reached together. lcov is
//! line-oriented, so reading one takes no dependency beyond splitting text.

use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::io::{self, Write as _};
use std::process::ExitCode;

/// Reminder of the task surface, printed with every usage error.
const USAGE: &str = "usage: cargo xtask coverage <lcov-file>...";

/// Line and branch counts read from the lcov records of one source file.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
struct Tally {
    /// Instrumented lines, from `LF` records.
    lines_found: u32,
    /// Instrumented lines a test reached, from `LH` records.
    lines_hit: u32,
    /// Instrumented branches, from `BRF` records.
    branches_found: u32,
    /// Instrumented branches a test reached, from `BRH` records.
    branches_hit: u32,
}

impl Tally {
    /// Whether the tests reached every instrumented line and branch counted
    /// here.
    const fn is_complete(&self) -> bool {
        self.lines_hit == self.lines_found && self.branches_hit == self.branches_found
    }

    /// Add `other` into this tally. The counts stop at the largest value they
    /// can hold rather than wrapping, so an implausibly large report reads as a
    /// shortfall instead of as a total that clears the bar.
    const fn absorb(&mut self, other: &Self) {
        self.lines_found = self.lines_found.saturating_add(other.lines_found);
        self.lines_hit = self.lines_hit.saturating_add(other.lines_hit);
        self.branches_found = self.branches_found.saturating_add(other.branches_found);
        self.branches_hit = self.branches_hit.saturating_add(other.branches_hit);
    }
}

/// Accumulate the records of one lcov tracefile into `totals`, keyed by the
/// source path each `SF` record names. Records the format defines and this
/// gate has no use for, per-line and per-branch hit counts among them, pass by
/// unread.
fn parse_lcov(text: &str, totals: &mut BTreeMap<String, Tally>) -> Result<(), String> {
    let mut current: Option<String> = None;
    for line in text.lines() {
        let Some((tag, value)) = line.split_once(':') else {
            continue;
        };
        match tag {
            "SF" => current = Some(value.to_owned()),
            "LF" | "LH" | "BRF" | "BRH" => {
                let path = current
                    .as_ref()
                    .ok_or_else(|| format!("a {tag} record arrives before any source file"))?;
                let count: u32 = value
                    .parse()
                    .map_err(|err| format!("unreadable {tag} count {value:?}: {err}"))?;
                let tally = totals.entry(path.clone()).or_default();
                let slot = match tag {
                    "LF" => &mut tally.lines_found,
                    "LH" => &mut tally.lines_hit,
                    "BRF" => &mut tally.branches_found,
                    _ => &mut tally.branches_hit,
                };
                *slot = slot.saturating_add(count);
            }
            _ => {}
        }
    }
    Ok(())
}

/// Read every tracefile in `paths` and merge their records into one map.
fn collect(paths: &[String]) -> Result<BTreeMap<String, Tally>, String> {
    if paths.is_empty() {
        return Err(format!("name at least one tracefile; {USAGE}"));
    }
    let mut totals = BTreeMap::new();
    for path in paths {
        let text = fs::read_to_string(path).map_err(|err| format!("{path}: {err}"))?;
        parse_lcov(&text, &mut totals)?;
    }
    Ok(totals)
}

/// Write the per-file table and the total to `out`, then fail when the total
/// leaves any line or any branch unreached. An empty map fails for the same
/// reason, since a report holding no records would otherwise clear the
/// threshold by carrying nothing to compare against. A total finding no
/// branch anywhere fails on that reading too: this tree has conditions in it
/// and a report saying otherwise lost its branch records on the way here.
fn report(totals: &BTreeMap<String, Tally>, out: &mut impl io::Write) -> Result<(), String> {
    if totals.is_empty() {
        return Err("the tracefiles carry no coverage records".to_owned());
    }
    let width = totals.keys().map(String::len).max().unwrap_or(0);
    let mut summary = Tally::default();
    for (path, tally) in totals {
        summary.absorb(tally);
        write_row(out, width, path, tally)?;
    }
    write_row(out, width, "total", &summary)?;
    if summary.branches_found == 0 {
        return Err("the records carry no branches at all".to_owned());
    }
    if summary.is_complete() {
        Ok(())
    } else {
        Err("the total leaves a line or a branch unreached".to_owned())
    }
}

/// Write one row of the table. A row carries a label and the two
/// hit-over-found counts, plus a marker where something went unreached.
fn write_row(
    out: &mut impl io::Write,
    width: usize,
    label: &str,
    tally: &Tally,
) -> Result<(), String> {
    let marker = if tally.is_complete() { "" } else { "  MISSING" };
    writeln!(
        out,
        "{label:<width$}  lines {}/{}  branches {}/{}{marker}",
        tally.lines_hit, tally.lines_found, tally.branches_hit, tally.branches_found,
    )
    .map_err(|err| format!("writing the coverage table failed: {err}"))
}

/// Dispatch the task named by the first argument.
fn run(args: &[String], out: &mut impl io::Write) -> Result<(), String> {
    let Some((task, rest)) = args.split_first() else {
        return Err(format!("name a task; {USAGE}"));
    };
    match task.as_str() {
        "coverage" => report(&collect(rest)?, out),
        other => Err(format!("unknown task {other:?}; {USAGE}")),
    }
}

fn main() -> ExitCode {
    let args: Vec<String> = env::args().skip(1).collect();
    let mut out = io::stdout().lock();
    match run(&args, &mut out) {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            // A write to stderr that itself fails leaves nowhere to report the
            // failure, so the result goes unread on purpose.
            #[expect(
                clippy::let_underscore_must_use,
                reason = "the exit code already carries the failure"
            )]
            let _ = writeln!(io::stderr(), "xtask: {message}");
            ExitCode::FAILURE
        }
    }
}

// A test asserts what it expects and stops at the first surprise, so
// unwrapping through expect is the point rather than a hazard. Production code
// answers to both lints.
#[cfg(test)]
#[expect(
    clippy::expect_used,
    reason = "a failed assumption in a test should end the test"
)]
mod tests {
    use std::collections::BTreeMap;

    use super::{Tally, collect, parse_lcov, report, run};

    const COMPLETE: &str = "\
SF:src/lexer.rs
DA:7,3
LF:12
LH:12
BRF:4
BRH:4
end_of_record
";

    const BRANCH_SHORTFALL: &str = "\
SF:src/lexer.rs
LF:12
LH:12
BRF:4
BRH:3
end_of_record
";

    const LINE_SHORTFALL: &str = "\
SF:src/lexer.rs
LF:12
LH:11
BRF:4
BRH:4
end_of_record
";

    const NO_BRANCH_RECORDS: &str = "\
SF:src/lexer.rs
LF:12
LH:12
end_of_record
";

    const TWO_FILES: &str = "\
SF:src/lexer.rs
LF:12
LH:12
BRF:4
BRH:4
end_of_record
SF:src/parser.rs
LF:20
LH:20
BRF:6
BRH:6
end_of_record
";

    fn tallies(text: &str) -> BTreeMap<String, Tally> {
        let mut totals = BTreeMap::new();
        parse_lcov(text, &mut totals).expect("the fixture parses");
        totals
    }

    fn rendered(text: &str) -> (String, Result<(), String>) {
        let mut out = Vec::new();
        let verdict = report(&tallies(text), &mut out);
        let table = String::from_utf8(out).expect("the table is utf-8");
        (table, verdict)
    }

    #[test]
    fn a_complete_tracefile_passes_and_prints_its_counts() {
        let (table, verdict) = rendered(COMPLETE);
        verdict.expect("a complete tracefile passes");
        assert!(table.contains("src/lexer.rs"));
        assert!(table.contains("lines 12/12  branches 4/4"));
        assert!(table.contains("total"));
        assert!(!table.contains("MISSING"));
    }

    #[test]
    fn an_unreached_branch_fails() {
        let err = rendered(BRANCH_SHORTFALL)
            .1
            .expect_err("a missed branch fails");
        assert!(err.contains("unreached"));
    }

    #[test]
    fn an_unreached_line_fails() {
        let err = rendered(LINE_SHORTFALL).1.expect_err("a missed line fails");
        assert!(err.contains("unreached"));
    }

    #[test]
    fn records_from_several_files_add_into_one_total() {
        let (table, verdict) = rendered(TWO_FILES);
        verdict.expect("both files are complete");
        assert!(table.contains("src/parser.rs"));
        assert!(table.contains("lines 32/32  branches 10/10"));
    }

    #[test]
    fn a_tracefile_stripped_of_its_branch_records_fails() {
        let err = rendered(NO_BRANCH_RECORDS)
            .1
            .expect_err("branches have to be there");
        assert!(err.contains("no branches at all"));
    }

    #[test]
    fn a_tracefile_carrying_no_records_fails() {
        let mut out = Vec::new();
        let err = report(&BTreeMap::new(), &mut out).expect_err("an empty report fails");
        assert!(err.contains("no coverage records"));
        assert!(out.is_empty());
    }

    #[test]
    fn naming_no_tracefile_fails() {
        let err = collect(&[]).expect_err("the task needs a tracefile");
        assert!(err.contains("name at least one tracefile"));
    }

    #[test]
    fn a_tracefile_that_does_not_exist_fails() {
        let paths = vec!["no-such-file.info".to_owned()];
        let err = collect(&paths).expect_err("a missing tracefile fails");
        assert!(err.contains("no-such-file.info"));
    }

    #[test]
    fn a_count_before_any_source_file_fails() {
        let mut totals = BTreeMap::new();
        let err = parse_lcov("LF:3\n", &mut totals).expect_err("a stray count fails");
        assert!(err.contains("before any source file"));
    }

    #[test]
    fn an_unreadable_count_fails() {
        let mut totals = BTreeMap::new();
        let err = parse_lcov("SF:src/lexer.rs\nLH:many\n", &mut totals)
            .expect_err("a count that is not a number fails");
        assert!(err.contains("unreadable LH count"));
    }

    #[test]
    fn an_unknown_task_fails() {
        let mut out = Vec::new();
        let args = vec!["mutants".to_owned()];
        let err = run(&args, &mut out).expect_err("only coverage is a task");
        assert!(err.contains("unknown task"));
    }

    #[test]
    fn naming_no_task_fails() {
        let mut out = Vec::new();
        let err = run(&[], &mut out).expect_err("the runner needs a task");
        assert!(err.contains("name a task"));
    }
}
