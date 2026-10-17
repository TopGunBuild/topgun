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
    // The contract names it `iter`; it returns the iterator inside a
    // `KvResult` because opening it can fail.
    #[allow(clippy::iter_not_returning_iterator)]
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
        let mut batch = WriteBatch::new();
        batch.delete(key);
        self.write(batch, opts)
    }

    /// Removes every key `k` with `start <= k < end`, atomically, as a one-op
    /// batch (see [`KvEngine::write`]). The cost is stated by
    /// [`KvCapabilities::range_delete_cost`].
    ///
    /// # Errors
    ///
    /// As [`KvEngine::write`]; [`KvError::InvalidArgument`] when `start > end`.
    fn delete_range(&self, start: &[u8], end: &[u8], opts: WriteOptions) -> KvResult<()> {
        let mut batch = WriteBatch::new();
        batch.delete_range(start, end);
        self.write(batch, opts)
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
        // The snapshot comes first, so a shut-down engine answers `ShutDown`
        // whatever the arguments.
        let snapshot = self.snapshot()?;
        if limit == 0 {
            return Err(KvError::InvalidArgument("scan_prefix limit is 0"));
        }
        let lower = match resume_after {
            Some(r) => {
                let mut after = Vec::with_capacity(r.len() + 1);
                after.extend_from_slice(r);
                after.push(0x00);
                after.max(prefix.to_vec())
            }
            None => prefix.to_vec(),
        };
        let upper = prefix_successor(prefix);
        if upper.as_ref().is_some_and(|u| lower >= *u) {
            return Ok(PrefixPage::default());
        }
        let entries = snapshot
            .iter(KeyRange {
                start: Some(lower),
                end: upper,
            })?
            .take(limit)
            .collect::<KvResult<Vec<_>>>()?;
        let resume_after = if entries.len() == limit {
            entries.last().map(|(key, _)| key.clone())
        } else {
            None
        };
        Ok(PrefixPage {
            entries,
            resume_after,
        })
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
    let mut successor = prefix.to_vec();
    while successor.last() == Some(&0xFF) {
        successor.pop();
    }
    let last = successor.last_mut()?;
    *last += 1;
    Some(successor)
}

#[cfg(test)]
pub(crate) use mem::MemKvEngine;

/// The reference engine: an ordered map under one lock. Test-only; it also
/// carries the crash hooks the crash suite drives.
#[cfg(test)]
mod mem {
    use std::collections::BTreeMap;
    use std::sync::mpsc::{channel, Receiver, Sender};
    use std::sync::Arc;

    use parking_lot::Mutex;

    use super::{
        BatchOp, KeyRange, KvCapabilities, KvEngine, KvError, KvIter, KvResult, KvSnapshot,
        RangeDeleteCost, WriteBatch, WriteOptions,
    };

    type Map = BTreeMap<Vec<u8>, Vec<u8>>;

    /// The in-memory reference engine.
    ///
    /// Besides the live map it keeps what a crash would leave: the `durable`
    /// map and the batches that returned `Ok` unsynced since the last fold.
    pub(crate) struct MemKvEngine {
        state: Mutex<State>,
    }

    struct State {
        live: Map,
        durable: Map,
        unsynced: Vec<Vec<BatchOp>>,
        /// Counted calls entered so far.
        calls: u64,
        /// The index of the counted call that kills the engine.
        death_at: Option<u64>,
        dead: bool,
        shut_down: bool,
        /// Armed by `park_between_ops`, taken by the next write of two or more ops.
        park: Option<Park>,
    }

    /// A one-shot stop between the first and the second op of a write, so a
    /// test can read while a batch is half applied — the correct `write`
    /// holds the lock there, so no reader can.
    struct Park {
        reached: Sender<()>,
        release: Receiver<()>,
    }

    impl Park {
        /// Announces the stop, then waits for a release or for the release
        /// sender to be dropped, so a failed test never leaves the writer blocked.
        fn wait(&self) {
            let _ = self.reached.send(());
            let _ = self.release.recv();
        }
    }

    impl State {
        fn fresh(map: Map) -> Self {
            Self {
                live: map.clone(),
                durable: map,
                unsynced: Vec::new(),
                calls: 0,
                death_at: None,
                dead: false,
                shut_down: false,
                park: None,
            }
        }

        /// Entry of a counted call: counted first, whatever it returns, then
        /// the death latch, then the shut-down check.
        fn enter_counted(&mut self) -> KvResult<()> {
            let index = self.calls;
            self.calls += 1;
            if !self.dead && self.death_at.is_some_and(|n| index >= n) {
                self.dead = true;
            }
            self.enter()
        }

        /// Entry of any call that returns a `KvResult`.
        fn enter(&self) -> KvResult<()> {
            if self.dead {
                return Err(KvError::Other(anyhow::anyhow!("mem engine is dead")));
            }
            if self.shut_down {
                return Err(KvError::ShutDown);
            }
            Ok(())
        }

        /// Applies a validated batch live and records it for durability;
        /// `park` stops it between its first and second op.
        fn commit(&mut self, operations: Vec<BatchOp>, opts: WriteOptions, park: Option<&Park>) {
            for (index, op) in operations.iter().enumerate() {
                if index == 1 {
                    if let Some(park) = park {
                        park.wait();
                    }
                }
                apply(&mut self.live, std::slice::from_ref(op));
            }
            self.unsynced.push(operations);
            if opts.sync {
                self.fold();
            }
        }

        /// Makes every listed batch durable.
        fn fold(&mut self) {
            for ops in self.unsynced.drain(..) {
                apply(&mut self.durable, &ops);
            }
        }
    }

    fn apply(map: &mut Map, ops: &[BatchOp]) {
        for op in ops {
            match op {
                BatchOp::Put { key, value } => {
                    map.insert(key.clone(), value.clone());
                }
                BatchOp::Delete { key } => {
                    map.remove(key);
                }
                BatchOp::DeleteRange { start, end } => {
                    if start < end {
                        let doomed: Vec<Vec<u8>> = map
                            .range::<Vec<u8>, _>(start..end)
                            .map(|(key, _)| key.clone())
                            .collect();
                        for key in doomed {
                            map.remove(&key);
                        }
                    }
                }
            }
        }
    }

    fn validate(ops: &[BatchOp]) -> KvResult<()> {
        for op in ops {
            if let BatchOp::DeleteRange { start, end } = op {
                if start > end {
                    return Err(KvError::InvalidArgument("delete range start > end"));
                }
            }
        }
        Ok(())
    }

    impl MemKvEngine {
        /// A fresh, empty, live engine.
        pub(crate) fn new() -> Self {
            Self {
                state: Mutex::new(State::fresh(Map::new())),
            }
        }

        /// Number of counted calls entered so far; 0 on a fresh engine or a fresh crash image.
        pub(crate) fn calls_so_far(&self) -> u64 {
            self.state.lock().calls
        }

        /// The counted call with index `at_call` and every later `KvEngine` / `KvSnapshot` call that returns a
        /// `KvResult` fail.
        pub(crate) fn arm_death(&self, at_call: u64) {
            self.state.lock().death_at = Some(at_call);
        }

        /// A fresh engine holding `durable` plus the first `keep` unsynced batches, each whole.
        pub(crate) fn crash_image(&self, keep: usize) -> MemKvEngine {
            let state = self.state.lock();
            let mut image = state.durable.clone();
            for ops in state.unsynced.iter().take(keep) {
                apply(&mut image, ops);
            }
            MemKvEngine {
                state: Mutex::new(State::fresh(image)),
            }
        }

        /// One-shot park between the first and the second op of the next `write` with at least two ops.
        ///
        /// That write sends `()` on the returned receiver's channel, then
        /// waits until a message arrives on the returned sender or the sender
        /// is dropped. It parks wherever the lock is at that point: the
        /// correct `write` holds it for the whole batch.
        pub(crate) fn park_between_ops(&self) -> (Receiver<()>, Sender<()>) {
            let (reached, reached_rx) = channel();
            let (release, release_rx) = channel();
            self.state.lock().park = Some(Park {
                reached,
                release: release_rx,
            });
            (reached_rx, release)
        }
    }

    impl KvEngine for MemKvEngine {
        fn capabilities(&self) -> KvCapabilities {
            KvCapabilities {
                engine: "mem",
                ingest: true,
                range_delete_cost: RangeDeleteCost::LinearInKeys,
            }
        }

        fn get(&self, key: &[u8]) -> KvResult<Option<Vec<u8>>> {
            let state = self.state.lock();
            state.enter()?;
            Ok(state.live.get(key).cloned())
        }

        fn write(&self, batch: WriteBatch, opts: WriteOptions) -> KvResult<()> {
            // One lock acquisition for the whole batch: no snapshot can be
            // taken between two of its ops (`TG-ENG-001`).
            let mut state = self.state.lock();
            state.enter_counted()?;
            let operations = batch.into_ops();
            validate(&operations)?;
            let park = if operations.len() >= 2 {
                state.park.take()
            } else {
                None
            };
            state.commit(operations, opts, park.as_ref());
            Ok(())
        }

        fn snapshot(&self) -> KvResult<Box<dyn KvSnapshot + '_>> {
            let state = self.state.lock();
            state.enter()?;
            Ok(Box::new(MemSnapshot {
                engine: self,
                map: Arc::new(state.live.clone()),
            }))
        }

        fn ingest(
            &self,
            range: KeyRange,
            sorted: Vec<(Vec<u8>, Vec<u8>)>,
            opts: WriteOptions,
        ) -> KvResult<()> {
            let mut state = self.state.lock();
            state.enter_counted()?;
            let (Some(start), Some(end)) = (range.start, range.end) else {
                return Err(KvError::InvalidArgument("ingest range must be bounded"));
            };
            if start > end {
                return Err(KvError::InvalidArgument("ingest range start > end"));
            }
            if !sorted.windows(2).all(|w| w[0].0 < w[1].0) {
                return Err(KvError::InvalidArgument(
                    "ingest input is not strictly ascending",
                ));
            }
            if !sorted.iter().all(|(key, _)| *key >= start && *key < end) {
                return Err(KvError::InvalidArgument("ingest key outside its range"));
            }
            let mut replace = Vec::with_capacity(sorted.len() + 1);
            replace.push(BatchOp::DeleteRange { start, end });
            replace.extend(
                sorted
                    .into_iter()
                    .map(|(key, value)| BatchOp::Put { key, value }),
            );
            state.commit(replace, opts, None);
            Ok(())
        }

        fn sync(&self) -> KvResult<()> {
            let mut state = self.state.lock();
            state.enter_counted()?;
            state.fold();
            Ok(())
        }

        fn shutdown(&self) -> KvResult<()> {
            let mut state = self.state.lock();
            state.enter_counted()?;
            state.fold();
            state.shut_down = true;
            Ok(())
        }
    }

    /// A snapshot of [`MemKvEngine`]: an immutable copy of the live map,
    /// taken under the engine's lock.
    struct MemSnapshot<'e> {
        /// Read only for the death latch, which covers snapshot calls too.
        engine: &'e MemKvEngine,
        map: Arc<Map>,
    }

    impl MemSnapshot<'_> {
        fn enter(&self) -> KvResult<()> {
            if self.engine.state.lock().dead {
                return Err(KvError::Other(anyhow::anyhow!("mem engine is dead")));
            }
            Ok(())
        }
    }

    impl KvSnapshot for MemSnapshot<'_> {
        fn get(&self, key: &[u8]) -> KvResult<Option<Vec<u8>>> {
            self.enter()?;
            Ok(self.map.get(key).cloned())
        }

        fn iter(&self, range: KeyRange) -> KvResult<KvIter<'_>> {
            use std::ops::Bound;
            self.enter()?;
            if let (Some(start), Some(end)) = (&range.start, &range.end) {
                if start > end {
                    return Err(KvError::InvalidArgument("iter range start > end"));
                }
            }
            let lower = range.start.map_or(Bound::Unbounded, Bound::Included);
            let upper = range.end.map_or(Bound::Unbounded, Bound::Excluded);
            Ok(Box::new(
                self.map
                    .range::<Vec<u8>, _>((lower, upper))
                    .map(|(key, value)| Ok((key.clone(), value.clone()))),
            ))
        }
    }
}
