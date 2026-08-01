// SPDX-License-Identifier: Apache-2.0
// Copyright Authors of Proofhouse

//! The synchronization primitives the concurrent modules build on,
//! named in one place so a checked build can put models in their seats.
//!
//! An ordinary build takes the standard library's types. A build
//! carrying `--cfg loom` takes loom's, which look the same to a caller
//! and record every atomic access, every lock, and every thread switch
//! so the model runner can replay the code under each ordering the
//! memory model permits. Routing the imports through this module is
//! what keeps that swap from touching the code that uses them.
//!
//! Scoped threads have no counterpart here: loom models thread handles
//! and not the borrow-bounded form, so the driver that fans work out
//! reaches for the standard library directly and the models cover the
//! primitives underneath it instead.

#![expect(
    clippy::redundant_pub_crate,
    reason = "the crate-wide marker is what keeps these names out of the published surface, which `unreachable_pub` would otherwise report on a plain `pub` here"
)]

#[cfg(loom)]
pub(crate) use loom::sync::atomic::{AtomicU8, AtomicUsize, Ordering};
#[cfg(loom)]
pub(crate) use loom::sync::{Arc, Mutex, MutexGuard};
#[cfg(loom)]
pub(crate) use loom::thread::yield_now;
#[cfg(not(loom))]
pub(crate) use std::sync::atomic::{AtomicU8, AtomicUsize, Ordering};
#[cfg(not(loom))]
pub(crate) use std::sync::{Arc, Mutex, MutexGuard};
#[cfg(not(loom))]
pub(crate) use std::thread::yield_now;
