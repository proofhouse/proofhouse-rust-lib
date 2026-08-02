// SPDX-License-Identifier: Apache-2.0
// Copyright Authors of Proofhouse

//! A cell any number of threads may reach for and one of them fills.

use crate::sync_shim::{AtomicU8, Mutex, MutexGuard, Ordering, yield_now};

/// Phase of a cell no thread has claimed yet.
const EMPTY: u8 = 0;

/// Phase of a cell one thread holds while it works the value out.
const INITIALIZING: u8 = 1;

/// Phase of a cell whose value sits in the slot for anyone to read.
const READY: u8 = 2;

/// A cell that takes a value once and hands out copies of it after.
///
/// The phases run one way and never back: empty, then claimed by the
/// thread working the value out, then ready with the value in the
/// slot. A flag holds the phase and a lock holds the slot, which
/// divides the labor rather than doubling it. The flag answers a
/// thread that waits on nobody, and the lock keeps a half-written slot
/// out of view.
///
/// The claim is a compare-and-exchange from empty, so exactly one of
/// the threads arriving at an unfilled cell runs the initializer and
/// the rest wait for its answer. Publication stores the ready flag
/// with `Release`, and every read of that flag loads it with
/// `Acquire`. That pair carries the guarantee the type rests on: a
/// thread seeing the ready flag sees everything the publishing thread
/// wrote beforehand. [`OnceValue::is_published`] hands the guarantee
/// to a caller who wants the answer without taking the lock, and
/// weakening either half would let such a caller read the
/// announcement ahead of what it announces.
pub struct OnceValue<T> {
    /// Which phase the cell stands in, and the one field a reader may
    /// consult without blocking.
    state: AtomicU8,
    /// Where the value waits, empty until the thread holding the claim
    /// puts it there.
    slot: Mutex<Option<T>>,
}

impl<T: Clone> OnceValue<T> {
    /// A cell with nothing in it.
    #[must_use]
    #[expect(
        clippy::missing_const_for_fn,
        reason = "the model runner's stand-ins for these primitives have no const constructor, and the same source builds against both"
    )]
    pub fn new() -> Self {
        Self {
            state: AtomicU8::new(EMPTY),
            slot: Mutex::new(None),
        }
    }

    /// Whether the value has landed.
    ///
    /// A `true` answer carries the writes the publishing thread made
    /// before it published, so a caller may go on to read data the
    /// publication announces without taking the lock this cell holds.
    /// A `false` answer says only that the value was still on its way
    /// at the moment of the read.
    #[must_use]
    pub fn is_published(&self) -> bool {
        self.state.load(Ordering::Acquire) == READY
    }

    /// A copy of the value, or `None` while the cell stands empty or
    /// the thread that claimed it works.
    #[must_use]
    pub fn get(&self) -> Option<T> {
        if self.is_published() {
            self.locked().clone()
        } else {
            None
        }
    }

    /// A copy of the value, running `init` to produce it when no other
    /// thread has.
    ///
    /// The claim comes first, an exchange reading the flag as much as
    /// writing it, so a thread finding the cell taken learns from the
    /// failure what a separate load would have told it. Whoever takes
    /// the claim runs `init` outside the lock, so an initializer may
    /// reach for other cells without meeting itself on the way. Every
    /// other thread reads the value out rather than working a second
    /// one out.
    pub fn get_or_init<F>(&self, init: F) -> T
    where
        F: FnOnce() -> T,
    {
        while !self.claim() {
            if let Some(value) = self.get() {
                return value;
            }
            // The claim belongs to another thread and its value has
            // not landed yet. Step aside for it.
            yield_now();
        }
        let value = init();
        let published = value.clone();
        *self.locked() = Some(published);
        self.state.store(READY, Ordering::Release);
        value
    }

    /// Try to move the cell from empty to claimed, reporting whether
    /// this thread took it.
    fn claim(&self) -> bool {
        self.state
            .compare_exchange(EMPTY, INITIALIZING, Ordering::Acquire, Ordering::Relaxed)
            .is_ok()
    }

    /// Take the lock on the slot, stepping past a poison flag.
    ///
    /// A panic under the lock raises that flag. Each critical section
    /// here moves one value in or copies one out, so only a `T` whose
    /// own clone panicked can raise it, and what sits behind the flag
    /// stays an `Option` either way: readable, writable, and none the
    /// worse for the panic that raised it.
    fn locked(&self) -> MutexGuard<'_, Option<T>> {
        match self.slot.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        }
    }
}

impl<T: Clone> Default for OnceValue<T> {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::OnceValue;
    use std::panic::{AssertUnwindSafe, catch_unwind};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::mpsc::{Receiver, Sender, channel};
    use std::thread;
    use std::time::Duration;

    /// How many copies of [`Fragile`] the tests have taken, which the
    /// one test needing a clone to fail on cue reads.
    static FRAGILE_CLONES: AtomicUsize = AtomicUsize::new(0);

    /// A value whose second copy panics, which is the one way a caller
    /// can poison the lock a cell keeps over its slot.
    #[derive(Debug, PartialEq, Eq)]
    struct Fragile;

    impl Clone for Fragile {
        fn clone(&self) -> Self {
            assert!(
                FRAGILE_CLONES.fetch_add(1, Ordering::Relaxed) != 1,
                "the second copy fails on purpose"
            );
            Self
        }
    }

    #[test]
    fn an_empty_cell_reports_nothing() {
        let cell: OnceValue<u8> = OnceValue::new();
        assert!(!cell.is_published());
        assert_eq!(cell.get(), None);
    }

    #[test]
    fn a_default_cell_is_an_empty_one() {
        let cell: OnceValue<u8> = OnceValue::default();
        assert_eq!(cell.get(), None);
    }

    /// Fill `cell` from a thread that keeps the claim until a message
    /// releases it, which is how the test that follows finds a second
    /// thread a claim to wait on.
    fn publish_on_cue(cell: &OnceValue<u8>, claimed: &Sender<()>, may_finish: &Receiver<()>) -> u8 {
        cell.get_or_init(|| {
            claimed.send(()).unwrap();
            may_finish.recv().unwrap();
            7
        })
    }

    #[test]
    fn the_first_initializer_decides_the_value() {
        let cell: OnceValue<u8> = OnceValue::new();
        assert_eq!(cell.get_or_init(|| 7), 7);
        assert!(cell.is_published());
        assert_eq!(cell.get(), Some(7));
        assert_eq!(cell.get_or_init(|| 9), 7);
    }

    #[test]
    fn a_waiting_thread_reads_what_the_winner_published() {
        let cell: OnceValue<u8> = OnceValue::new();
        let shared = &cell;
        let (claimed, claim_reached) = channel();
        let (finish, may_finish) = channel();
        let (waiting, wait_reached) = channel();
        thread::scope(|scope| {
            scope.spawn(move || publish_on_cue(shared, &claimed, &may_finish));
            claim_reached.recv().unwrap();
            let waiter = scope.spawn(move || {
                waiting.send(()).unwrap();
                shared.get_or_init(|| 9)
            });
            // The winner holds the claim until the message below, and
            // the second thread speaks up on the line before it reaches
            // the cell. The pause then covers those few instructions
            // rather than the thread start ahead of them, and the
            // release finds the second thread in the wait loop rather
            // than past it on the published value.
            wait_reached.recv().unwrap();
            thread::sleep(Duration::from_millis(20));
            finish.send(()).unwrap();
            assert_eq!(waiter.join().unwrap(), 7);
        });
        assert_eq!(cell.get(), Some(7));
    }

    #[test]
    fn a_panicking_copy_leaves_the_cell_readable() {
        let cell = OnceValue::new();
        assert_eq!(cell.get_or_init(|| Fragile), Fragile);
        let poisoning = catch_unwind(AssertUnwindSafe(|| cell.get()));
        poisoning.unwrap_err();
        assert_eq!(cell.get(), Some(Fragile));
    }
}
