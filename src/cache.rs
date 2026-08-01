// SPDX-License-Identifier: Apache-2.0
// Copyright Authors of Proofhouse

//! A table of expression values many threads may fill at once.

use crate::ast::Expr;
use crate::errors::EvalError;
use crate::evaluator::{Rational, evaluate};
use crate::formatter::format_expr;
use crate::sync::OnceValue;
use crate::sync_shim::{Arc, AtomicUsize, Mutex, MutexGuard, Ordering};
use std::collections::HashMap;

/// One entry of the table, shared out by handle so a thread can wait on
/// a value without holding the table itself.
type Entry = Arc<OnceValue<Result<Rational, EvalError>>>;

/// Values already worked out, kept under the canonical text of the
/// expression that produced them.
///
/// Trees of one shape render one way, which is what makes a rendering
/// the key: it names the subexpression rather than the node object, so
/// a repeat of a subexpression finds the entry the first one left
/// behind wherever the walk meets it. A failure counts as an answer
/// and takes the same place, so an expression dividing by zero costs
/// one attempt.
///
/// Each entry is an [`OnceValue`], so one thread works out a key the
/// rest are asking for and they wait on its result.
/// [`ExprCache::evaluations`] counts those computations, which turns
/// the promise into something a caller checks rather than assumes.
#[expect(
    clippy::module_name_repetitions,
    reason = "the module holds one type and the name says what it caches, which `cache::Cache` would drop"
)]
pub struct ExprCache {
    /// Canonical renderings against the cell holding each value.
    entries: Mutex<HashMap<String, Entry>>,
    /// How many entries have run their computation, rising once per
    /// key whatever number of threads asked for it.
    evaluations: AtomicUsize,
}

impl ExprCache {
    /// A table with nothing in it.
    #[must_use]
    pub fn new() -> Self {
        Self {
            entries: Mutex::new(HashMap::new()),
            evaluations: AtomicUsize::new(0),
        }
    }

    /// The value of `expr`, which the ordinary walk works out when the
    /// table holds no answer for the key yet.
    ///
    /// # Errors
    ///
    /// Returns whatever the walk reports, which the table then keeps
    /// under the key like any other answer.
    pub fn evaluate(&self, expr: &Expr) -> Result<Rational, EvalError> {
        self.get_or_compute(expr, || evaluate(expr))
    }

    /// How many keys the table has computed a value for.
    ///
    /// Reading this once the threads sharing a table finish says how
    /// much work the sharing saved, and says it exactly: the count
    /// rises once per key, inside the computation itself.
    #[must_use]
    pub fn evaluations(&self) -> usize {
        self.evaluations.load(Ordering::Relaxed)
    }

    /// The value stored under the canonical rendering of `expr`,
    /// running `compute` to produce it on the first ask.
    ///
    /// The ordinary walk answers a miss one way and the parallel
    /// driver answers it another, so the computation arrives from the
    /// caller and the table keeps the promise that stays its own: one
    /// run per key, whatever the key means to whoever filled it.
    pub(crate) fn get_or_compute<F>(&self, expr: &Expr, compute: F) -> Result<Rational, EvalError>
    where
        F: FnOnce() -> Result<Rational, EvalError>,
    {
        let entry = self.entry(&format_expr(expr));
        entry.get_or_init(|| {
            self.evaluations.fetch_add(1, Ordering::Relaxed);
            compute()
        })
    }

    /// The cell filed under `key`, adding an empty one when the key is
    /// new.
    ///
    /// The lookup alone holds the table lock. Whatever the cell then
    /// costs to fill falls outside it, which is what lets one
    /// computation ask the table about another.
    fn entry(&self, key: &str) -> Entry {
        let mut entries = self.locked();
        Arc::clone(
            entries
                .entry(key.to_owned())
                .or_insert_with(|| Arc::new(OnceValue::new())),
        )
    }

    /// Take the lock on the table, stepping past a poison flag for the
    /// reason [`OnceValue`] does: nothing under this lock does more
    /// than look a key up, and a table behind a poison flag serves as
    /// well as the one in front of it.
    fn locked(&self) -> MutexGuard<'_, HashMap<String, Entry>> {
        match self.entries.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        }
    }
}

impl Default for ExprCache {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::ExprCache;
    use crate::ast::{BinaryOperator, Expr};
    use crate::errors::EvalError;
    use crate::evaluator::Rational;
    use crate::parser::parse;
    use std::thread;

    /// Build the tree a text spells out, the cases below all being
    /// texts the parser accepts.
    fn tree(text: &str) -> Expr {
        parse(text).unwrap()
    }

    #[test]
    fn a_repeated_key_is_computed_once() {
        let cache = ExprCache::new();
        let expr = tree("1/3 + 1/6");
        let first = cache.evaluate(&expr);
        let second = cache.evaluate(&tree("1/3 + 1/6"));
        assert_eq!(first, Ok(Rational::new(1, 2).unwrap()));
        assert_eq!(second, first);
        assert_eq!(cache.evaluations(), 1);
    }

    #[test]
    fn distinct_keys_are_computed_apiece() {
        let cache = ExprCache::default();
        assert_eq!(
            cache.evaluate(&tree("2 + 3")),
            Ok(Rational::from_integer(5))
        );
        assert_eq!(
            cache.evaluate(&tree("2 * 3")),
            Ok(Rational::from_integer(6))
        );
        assert_eq!(cache.evaluations(), 2);
    }

    #[test]
    fn a_failure_is_stored_like_any_other_answer() {
        let cache = ExprCache::new();
        let expr = tree("1/0");
        assert_eq!(cache.evaluate(&expr), Err(EvalError::DivisionByZero));
        assert_eq!(cache.evaluate(&expr), Err(EvalError::DivisionByZero));
        assert_eq!(cache.evaluations(), 1);
    }

    #[test]
    fn a_caller_may_answer_a_miss_its_own_way() {
        let cache = ExprCache::new();
        let expr = Expr::Number(4);
        let computed = cache.get_or_compute(&expr, || Ok(Rational::from_integer(9)));
        assert_eq!(computed, Ok(Rational::from_integer(9)));
        assert_eq!(cache.evaluate(&expr), Ok(Rational::from_integer(9)));
        assert_eq!(cache.evaluations(), 1);
    }

    #[test]
    fn two_threads_on_one_key_evaluate_it_once() {
        let cache = ExprCache::new();
        let expr = Expr::BinaryOp {
            op: BinaryOperator::Add,
            left: Box::new(Expr::Number(20)),
            right: Box::new(Expr::Number(22)),
        };
        let answers = thread::scope(|scope| {
            let worker = scope.spawn(|| cache.evaluate(&expr));
            let here = cache.evaluate(&expr);
            (worker.join().unwrap(), here)
        });
        assert_eq!(answers.0, Ok(Rational::from_integer(42)));
        assert_eq!(answers.1, answers.0);
        assert_eq!(cache.evaluations(), 1);
    }
}
