// SPDX-License-Identifier: Apache-2.0
// Copyright Authors of Proofhouse

//! Interleaving models of the publish-once cell and the table over it.
//!
//! Each model runs under loom, which replays its closure once per
//! ordering the memory model allows the threads and the atomics inside
//! it, and reports the first replay that breaks an assertion. A model
//! of two threads and a handful of atomic accesses keeps that replay
//! count small enough to run on every change. The file compiles to
//! nothing without `--cfg loom`, so an ordinary test run walks past
//! it.

#![cfg(loom)]

use loom::sync::Arc;
use loom::sync::atomic::{AtomicUsize, Ordering};
use loom::thread;
use proofhouse_rust_lib::ast::{BinaryOperator, Expr};
use proofhouse_rust_lib::cache::ExprCache;
use proofhouse_rust_lib::evaluator::Rational;
use proofhouse_rust_lib::sync::OnceValue;

/// The tree the cache model asks about, rebuilt on each replay because
/// a model runs its body many times over.
fn shared_tree() -> Expr {
    Expr::BinaryOp {
        op: BinaryOperator::Add,
        left: Box::new(Expr::Number(20)),
        right: Box::new(Expr::Number(22)),
    }
}

#[test]
fn two_threads_racing_to_fill_a_cell_run_one_initializer() {
    loom::model(|| {
        let cell: Arc<OnceValue<u8>> = Arc::new(OnceValue::new());
        let runs = Arc::new(AtomicUsize::new(0));
        let worker_cell = Arc::clone(&cell);
        let worker_runs = Arc::clone(&runs);
        let worker = thread::spawn(move || {
            worker_cell.get_or_init(|| {
                worker_runs.fetch_add(1, Ordering::Relaxed);
                7
            })
        });
        let here = cell.get_or_init(|| {
            runs.fetch_add(1, Ordering::Relaxed);
            7
        });
        assert_eq!(worker.join().unwrap(), here);
        assert_eq!(runs.load(Ordering::Relaxed), 1, "the initializer ran twice");
    });
}

#[test]
fn a_published_cell_carries_what_the_writer_did_first() {
    loom::model(|| {
        let cell: Arc<OnceValue<u8>> = Arc::new(OnceValue::new());
        let prepared = Arc::new(AtomicUsize::new(0));
        let worker_cell = Arc::clone(&cell);
        let worker_prepared = Arc::clone(&prepared);
        let worker = thread::spawn(move || {
            worker_prepared.store(1, Ordering::Relaxed);
            worker_cell.get_or_init(|| 7);
        });
        // The write preceding the claim is ordinary shared data that
        // the publication announces. Loading the flag with `Acquire`
        // is what makes it visible here, and any weaker read leaves
        // this thread free to meet the announcement without the thing
        // it announces.
        if cell.is_published() {
            assert_eq!(
                prepared.load(Ordering::Relaxed),
                1,
                "the flag arrived ahead of the write it announces"
            );
        }
        worker.join().unwrap();
    });
}

#[test]
fn a_reader_meeting_a_cell_mid_flight_sees_no_half_value() {
    loom::model(|| {
        let cell: Arc<OnceValue<u8>> = Arc::new(OnceValue::new());
        let worker_cell = Arc::clone(&cell);
        let worker = thread::spawn(move || worker_cell.get_or_init(|| 7));
        let seen = cell.get();
        assert!(
            matches!(seen, None | Some(7)),
            "a reader saw {seen:?} rather than the whole value or nothing"
        );
        assert_eq!(worker.join().unwrap(), 7);
        assert_eq!(cell.get(), Some(7));
    });
}

#[test]
fn two_threads_asking_one_key_evaluate_it_once() {
    loom::model(|| {
        let cache = Arc::new(ExprCache::new());
        let worker_cache = Arc::clone(&cache);
        let worker = thread::spawn(move || worker_cache.evaluate(&shared_tree()));
        let here = cache.evaluate(&shared_tree());
        assert_eq!(here, Ok(Rational::from_integer(42)));
        assert_eq!(worker.join().unwrap(), here);
        assert_eq!(cache.evaluations(), 1, "the key was evaluated twice");
    });
}
