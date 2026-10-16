//! `KvEngine`: the byte-level boundary between the record layer and a local
//! storage engine.
//!
//! An engine stores arbitrary byte keys and values in one ordered key space and
//! offers exactly what a record layer needs from it: point reads, atomic write
//! batches, range deletes, point-in-time snapshots with ordered iteration, a
//! paged prefix scan, an optional bulk ingest, an explicit sync and a shutdown.
//! An operation an engine does not offer is refused by type
//! ([`KvError::Unsupported`]) and announced in advance by
//! [`KvEngine::capabilities`].
//!
//! The contract is written on each item below and is checked by one
//! conformance suite that every engine runs unchanged; an engine is correct
//! when it passes that suite, not when it resembles another engine
//! (`TG-ENG-001`, `TG-ENG-002`, `TG-ENG-003`).
//!
//! The boundary is synchronous: the engines behind it block (a commit waits
//! for its fsync), and an async signature would only hide that.

use crate::storage::StoreBrokenCause;

/// The result of every engine call that can fail.
pub type KvResult<T> = Result<T, KvError>;

/// Why an engine call failed.
#[derive(Debug, thiserror::Error)]
pub enum KvError {
    /// The engine reports itself broken for this process; `TG-WB-010` stops the node on it.
    #[error("engine broken ({cause:?}): {detail}")]
    Broken {
        /// What broke, as the store-broken path reports it.
        cause: StoreBrokenCause,
        /// The engine's own description, for the log only; never parsed.
        detail: String,
    },
    /// The engine does not offer this operation; nothing was changed.
    #[error("engine does not support {0:?}")]
    Unsupported(KvCapability),
    /// The arguments break the method's contract; nothing was changed.
    #[error("invalid argument: {0}")]
    InvalidArgument(&'static str),
    /// `shutdown` has returned; the engine serves nothing more.
    #[error("engine is shut down")]
    ShutDown,
    /// Any other engine error. Says nothing about the engine's health.
    #[error(transparent)]
    Other(anyhow::Error),
}

/// An operation that not every engine offers. Every other operation of
/// [`KvEngine`] is mandatory.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KvCapability {
    /// [`KvEngine::ingest`].
    Ingest,
}

/// How the cost of a range delete grows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RangeDeleteCost {
    /// Linear in the number of keys the range removes.
    LinearInKeys,
    /// Independent of the number of keys removed.
    Constant,
}

/// What an engine offers, stated before any call is made.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KvCapabilities {
    /// A short engine name for logs and metrics (`"mem"`, `"redb"`).
    pub engine: &'static str,
    /// Whether [`KvEngine::ingest`] is offered; when `false` it answers
    /// [`KvError::Unsupported`]`(`[`KvCapability::Ingest`]`)`.
    pub ingest: bool,
    /// The cost of [`KvEngine::delete_range`] and of a [`BatchOp::DeleteRange`].
    pub range_delete_cost: RangeDeleteCost,
}

/// One operation of a [`WriteBatch`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BatchOp {
    /// Sets `key` to `value`.
    Put {
        /// The key.
        key: Vec<u8>,
        /// The value.
        value: Vec<u8>,
    },
    /// Removes `key`; removing an absent key is not an error.
    Delete {
        /// The key.
        key: Vec<u8>,
    },
    /// Half-open `[start, end)`.
    DeleteRange {
        /// The least key removed.
        start: Vec<u8>,
        /// The least key above the range; kept.
        end: Vec<u8>,
    },
}

/// An ordered list of operations applied atomically by [`KvEngine::write`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WriteBatch {
    ops: Vec<BatchOp>,
}

impl WriteBatch {
    /// An empty batch.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Appends a put of `key` = `value`.
    pub fn put(&mut self, key: impl Into<Vec<u8>>, value: impl Into<Vec<u8>>) {
        self.ops.push(BatchOp::Put {
            key: key.into(),
            value: value.into(),
        });
    }

    /// Appends a delete of `key`.
    pub fn delete(&mut self, key: impl Into<Vec<u8>>) {
        self.ops.push(BatchOp::Delete { key: key.into() });
    }

    /// Appends a delete of every key in `[start, end)`.
    pub fn delete_range(&mut self, start: impl Into<Vec<u8>>, end: impl Into<Vec<u8>>) {
        self.ops.push(BatchOp::DeleteRange {
            start: start.into(),
            end: end.into(),
        });
    }

    /// The number of operations.
    #[must_use]
    pub fn len(&self) -> usize {
        self.ops.len()
    }

    /// Whether the batch holds no operation.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.ops.is_empty()
    }

    /// The operations, in batch order.
    #[must_use]
    pub fn ops(&self) -> &[BatchOp] {
        &self.ops
    }

    /// The operations, in batch order, by value.
    #[must_use]
    pub fn into_ops(self) -> Vec<BatchOp> {
        self.ops
    }
}

/// How a write is made durable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WriteOptions {
    /// `true`: the call returns `Ok` only once the batch, and every batch
    /// before it, is durable. `false`: the batch is visible at once and
    /// becomes durable with a later `sync = true` write, [`KvEngine::sync`] or
    /// [`KvEngine::shutdown`].
    pub sync: bool,
}

/// `start` inclusive when `Some`, `end` exclusive when `Some`; `None` is open.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct KeyRange {
    /// The least key included, or open below.
    pub start: Option<Vec<u8>>,
    /// The least key excluded above, or open above.
    pub end: Option<Vec<u8>>,
}

/// One page of [`KvEngine::scan_prefix`].
#[derive(Debug, Default, PartialEq, Eq)]
pub struct PrefixPage {
    /// The page's entries, ascending by key.
    pub entries: Vec<(Vec<u8>, Vec<u8>)>,
    /// The key of the last entry when more keys under the prefix may follow; `None` when exhausted.
    pub resume_after: Option<Vec<u8>>,
}

/// An ordered iterator over a snapshot's entries.
pub type KvIter<'s> = Box<dyn Iterator<Item = KvResult<(Vec<u8>, Vec<u8>)>> + Send + 's>;

/// A consistent point-in-time view of an engine, from [`KvEngine::snapshot`].
///
/// It contains every batch whose `write` returned before the snapshot was
/// taken, no batch whose `write` was called after it was taken, and each batch
/// whose `write` overlapped the call either wholly or not at all. It never
/// sees a put, delete or range delete made after it. Of two snapshots taken
/// one after the other by one thread, the later contains every batch the
/// earlier contains. A live snapshot pins engine resources (a copy-on-write
/// engine cannot reuse a page an open snapshot may read); holding one is the
/// caller's cost.
pub trait KvSnapshot: Send + Sync {
    /// The snapshot's value of `key`, or `None`.
    ///
    /// # Errors
    ///
    /// [`KvError::Broken`] or [`KvError::Other`] as the engine reports them.
    fn get(&self, key: &[u8]) -> KvResult<Option<Vec<u8>>>;

    /// The snapshot's entries with keys inside `range`, strictly ascending by
    /// unsigned byte-lexicographic order. Several iterators of one snapshot
    /// may be alive at once, and none is affected by writes made while it
    /// runs.
    ///
    /// # Errors
    ///
    /// [`KvError::InvalidArgument`] when `start > end`; otherwise as
    /// [`KvSnapshot::get`], also for each item.
    fn iter(&self, range: KeyRange) -> KvResult<KvIter<'_>>;
}

/// A local byte-level storage engine.
///
/// **Keys and order.** Keys and values are arbitrary byte strings, the empty
/// key included. Every ordered view (snapshot iterator, `scan_prefix`) is
/// strictly ascending by unsigned byte-lexicographic order (`[u8]`'s `Ord`):
/// `a` < `a\0` < `ab`.
///
/// **Durability.** A batch is *made durable* when a `write` of it with
/// `sync = true` returns `Ok`, or when a later `sync()` or a later
/// `sync = true` write returns `Ok`; a durable batch makes every batch written
/// before it durable. After a crash the durable state equals the result of
/// applying, in call order, a prefix of the batches whose `write` was called —
/// each batch wholly or not at all — and that prefix contains every batch made
/// durable before the crash. A batch whose `write` had not returned when the
/// crash happened may be in the prefix or not.
///
/// **Errors.** [`KvError::Broken`] only for what `TG-WB-010` treats as fatal
/// for this process: an I/O error, a previous I/O error, corruption, a
/// poisoned lock. The decision is made from a typed classification — the
/// engine's error type, or the error-kind enum its binding library exposes —
/// and never from error text. An engine whose classification carries no
/// [`std::io::ErrorKind`] reports an I/O error as
/// `Broken { cause: Io(ErrorKind::Other) }`. Everything else is
/// [`KvError::Other`]. No method panics on an input the contract admits.
///
/// **Shutdown.** Once [`KvEngine::shutdown`] has returned, every method that
/// returns a [`KvResult`] answers [`KvError::ShutDown`]; `capabilities` still
/// answers.
pub trait KvEngine: Send + Sync + 'static {
    /// What this engine offers. Answers also after `shutdown`.
    fn capabilities(&self) -> KvCapabilities;

    /// The value of the last write of `key` visible to this process, or `None`.
    ///
    /// # Errors
    ///
    /// [`KvError::ShutDown`] after `shutdown`; [`KvError::Broken`] or
    /// [`KvError::Other`] as the engine reports them.
    fn get(&self, key: &[u8]) -> KvResult<Option<Vec<u8>>>;

    /// Applies `batch` atomically and in order.
    ///
    /// Ops apply in batch order: a later op on the same key wins, and a range
    /// delete removes keys put earlier in the same batch. When the call
    /// returns `Ok`, every later `get`, `snapshot` and `scan_prefix` of this
    /// process sees all of the batch, and no reader ever observes a strict
    /// subset of it (`TG-ENG-001`). Durability follows `opts.sync` (see the
    /// trait's durability clause). A range delete with `start == end` removes
    /// nothing; an empty batch is `Ok` and changes nothing.
    ///
    /// # Errors
    ///
    /// [`KvError::InvalidArgument`] when a range delete has `start > end`, and
    /// then nothing of the batch is applied; [`KvError::ShutDown`] after
    /// `shutdown`; [`KvError::Broken`] or [`KvError::Other`] as the engine
    /// reports them, with nothing of the batch applied.
    fn write(&self, batch: WriteBatch, opts: WriteOptions) -> KvResult<()>;

    /// Removes `key`, as a one-op batch (see [`KvEngine::write`]).
    ///
    /// # Errors
    ///
    /// As [`KvEngine::write`].
    fn delete(&self, key: &[u8], opts: WriteOptions) -> KvResult<()> {
        let _ = (key, opts);
        unimplemented!()
    }

    /// Removes every key `k` with `start <= k < end`, atomically, as a one-op
    /// batch (see [`KvEngine::write`]). The cost is stated by
    /// [`KvCapabilities::range_delete_cost`].
    ///
    /// # Errors
    ///
    /// As [`KvEngine::write`]; [`KvError::InvalidArgument`] when `start > end`.
    fn delete_range(&self, start: &[u8], end: &[u8], opts: WriteOptions) -> KvResult<()> {
        let _ = (start, end, opts);
        unimplemented!()
    }

    /// A consistent point-in-time view (see [`KvSnapshot`]). The snapshot may
    /// borrow the engine, so an engine whose native snapshot borrows its
    /// database needs neither `unsafe` nor a self-referential type.
    ///
    /// # Errors
    ///
    /// [`KvError::ShutDown`] after `shutdown`; [`KvError::Broken`] or
    /// [`KvError::Other`] as the engine reports them.
    fn snapshot(&self) -> KvResult<Box<dyn KvSnapshot + '_>>;

    /// One page of the keys that start with `prefix` and are strictly greater
    /// than `resume_after` (when given), ascending, at most `limit` of them,
    /// read from one snapshot taken at the call.
    ///
    /// **Page rule.** The page's `resume_after` is `Some(last returned key)`
    /// if and only if the page holds exactly `limit` entries, else `None`; no
    /// engine looks ahead, so a walk whose last page is full ends with one
    /// more call that returns an empty page and `None`. The empty prefix means
    /// every key; a prefix ending in `0xFF` bytes is handled.
    ///
    /// **Walk rule.** A walk feeds each page's `resume_after` to the next
    /// call until `None`. A page's interval is the keys under the prefix
    /// greater than its input `resume_after` and, when the page is full, not
    /// greater than its last key. A walk returns a key at most once, and
    /// returns it iff it is present in the snapshot of the call whose page
    /// interval contains it: a key present for the whole walk is returned
    /// exactly once, a key put behind the cursor never, a key put ahead of it
    /// once, and deleting the cursor key changes nothing for later pages
    /// (`TG-ENG-002`).
    ///
    /// The default reads `snapshot()?.iter(lower..upper)` with
    /// `lower = max(prefix, r ‖ 0x00)` for `resume_after = Some(r)` (the least
    /// key greater than `r`), else `prefix`, and `upper` the prefix's
    /// successor (open when the prefix is empty or all `0xFF`).
    ///
    /// # Errors
    ///
    /// [`KvError::InvalidArgument`] when `limit == 0`; otherwise as
    /// [`KvEngine::snapshot`] and [`KvSnapshot::iter`].
    fn scan_prefix(
        &self,
        prefix: &[u8],
        resume_after: Option<&[u8]>,
        limit: usize,
    ) -> KvResult<PrefixPage> {
        let _ = (prefix, resume_after, limit);
        unimplemented!()
    }

    /// When [`KvCapabilities::ingest`] is `true`: atomically replaces the
    /// content of `range` by `sorted`; durability as for [`KvEngine::write`].
    /// `range` must be bounded at both ends, and `sorted` strictly ascending
    /// and inside `range`.
    ///
    /// # Errors
    ///
    /// When ingest is not offered: [`KvError::Unsupported`]`(`[`KvCapability::Ingest`]`)`
    /// whatever the arguments (only the shut-down check precedes it), with
    /// nothing changed. When it is: [`KvError::InvalidArgument`] for an
    /// open-bounded range, `start > end`, unsorted input or a key outside
    /// `range`, with nothing changed. [`KvError::ShutDown`] after `shutdown`,
    /// whatever the capability says.
    fn ingest(
        &self,
        range: KeyRange,
        sorted: Vec<(Vec<u8>, Vec<u8>)>,
        opts: WriteOptions,
    ) -> KvResult<()>;

    /// On `Ok`, every batch whose `write` returned before `sync` was called is
    /// durable.
    ///
    /// # Errors
    ///
    /// [`KvError::ShutDown`] after `shutdown`; [`KvError::Broken`] or
    /// [`KvError::Other`] as the engine reports them.
    fn sync(&self) -> KvResult<()>;

    /// Makes every returned batch durable (as `sync`), then stops the engine:
    /// from its return on, every method that returns a [`KvResult`] —
    /// `shutdown` and `ingest` included — answers [`KvError::ShutDown`]. The
    /// engine's files are released no later than when the engine value is
    /// dropped; after that a new engine opened on the same files succeeds and
    /// holds every batch whose `write` returned before `shutdown`
    /// (`TG-ENG-001`). Snapshots alive at the call may keep answering or
    /// answer `ShutDown`.
    ///
    /// # Errors
    ///
    /// [`KvError::ShutDown`] when already shut down; [`KvError::Broken`] or
    /// [`KvError::Other`] when the final sync fails, and then the engine stays
    /// open.
    fn shutdown(&self) -> KvResult<()>;
}

/// The least byte string greater than every string that starts with
/// `prefix`, or `None` when there is none (the empty prefix, or a prefix of
/// `0xFF` bytes only): every trailing `0xFF` is stripped and the last
/// remaining byte incremented, so no carry ever overflows.
fn prefix_successor(prefix: &[u8]) -> Option<Vec<u8>> {
    let _ = prefix;
    unimplemented!()
}

#[cfg(test)]
pub(crate) use mem::MemKvEngine;

/// The reference engine: an ordered map under one lock. Test-only; it also
/// carries the crash hooks the crash suite drives.
#[cfg(test)]
mod mem {
    use super::{
        KeyRange, KvCapabilities, KvEngine, KvIter, KvResult, KvSnapshot, WriteBatch, WriteOptions,
    };

    /// The in-memory reference engine.
    pub(crate) struct MemKvEngine {
        _private: (),
    }

    impl MemKvEngine {
        /// A fresh, empty, live engine.
        pub(crate) fn new() -> Self {
            unimplemented!()
        }

        /// Number of counted calls entered so far; 0 on a fresh engine or a fresh crash image.
        pub(crate) fn calls_so_far(&self) -> u64 {
            unimplemented!()
        }

        /// The counted call with index `at_call` and every later `KvEngine` / `KvSnapshot` call that returns a
        /// `KvResult` fail.
        pub(crate) fn arm_death(&self, at_call: u64) {
            let _ = at_call;
            unimplemented!()
        }

        /// A fresh engine holding `durable` plus the first `keep` unsynced batches, each whole.
        pub(crate) fn crash_image(&self, keep: usize) -> MemKvEngine {
            let _ = keep;
            unimplemented!()
        }
    }

    impl KvEngine for MemKvEngine {
        fn capabilities(&self) -> KvCapabilities {
            unimplemented!()
        }

        fn get(&self, key: &[u8]) -> KvResult<Option<Vec<u8>>> {
            let _ = key;
            unimplemented!()
        }

        fn write(&self, batch: WriteBatch, opts: WriteOptions) -> KvResult<()> {
            let _ = (batch, opts);
            unimplemented!()
        }

        fn snapshot(&self) -> KvResult<Box<dyn KvSnapshot + '_>> {
            unimplemented!()
        }

        fn ingest(
            &self,
            range: KeyRange,
            sorted: Vec<(Vec<u8>, Vec<u8>)>,
            opts: WriteOptions,
        ) -> KvResult<()> {
            let _ = (range, sorted, opts);
            unimplemented!()
        }

        fn sync(&self) -> KvResult<()> {
            unimplemented!()
        }

        fn shutdown(&self) -> KvResult<()> {
            unimplemented!()
        }
    }

    /// A snapshot of [`MemKvEngine`].
    struct MemSnapshot;

    impl KvSnapshot for MemSnapshot {
        fn get(&self, key: &[u8]) -> KvResult<Option<Vec<u8>>> {
            let _ = key;
            unimplemented!()
        }

        fn iter(&self, range: KeyRange) -> KvResult<KvIter<'_>> {
            let _ = range;
            unimplemented!()
        }
    }
}
