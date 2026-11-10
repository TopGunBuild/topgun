//! The writer of the WAL stream: one log-writer thread that owns the active
//! stream segment and writes the frames of every partition into it, fed by a
//! bounded channel, and one syncer thread that fsyncs what the writer wrote.
//!
//! Every frame goes through the one writer, so a group of concurrent appends
//! is one vectored write and is made durable by one fsync however many
//! partitions it spans. The fsync runs on the syncer, outside anything the
//! next write waits on: the writer goes on writing the next group while the
//! fsync of the previous one runs, and it never waits for an fsync, at
//! rotation included (ACCEPTOR §2.3, `TG-WAL-001`, `TG-WAL-002`).
//!
//! An append resolves:
//! - under `per_op`, once an fsync that started after its frame was written
//!   returned — the syncer publishes the position it fsynced up to, and only
//!   that publication resolves the append (acked implies durable);
//! - under `batched` and `none`, once its frame was written;
//! - under every policy, never before the segment it is in was published
//!   "open": its header, its directory entry and the tail of every earlier
//!   segment are durable (none of it under `none`, which never fsyncs).
//!
//! A failed write or fsync stops the process under every policy
//! (`TG-WAL-007` tier (B)): after a failed fsync the kernel may have dropped
//! the dirty pages and cleared the error, so no later fsync can vouch for them.

use std::collections::{BTreeMap, VecDeque};
use std::io::{IoSlice, Write as _};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use tokio::sync::{mpsc, oneshot, watch};

use super::segment::{
    self, FrameLocation, SegmentCipher, SegmentSummary, StreamSegmentHeader, STREAM_HEADER_LEN,
};
use super::{format, fsync_dir, wal_fail_stop, StreamSegment, WalFailStopTier, WalFsyncPolicy};
use crate::storage::sealing::{Keyring, SegmentKey};

/// Capacity of the submission channel, in commands. A producer that finds it
/// full waits for room (back-pressure into the caller); the depth and the age
/// of the oldest waiting submission are metrics.
pub(crate) const WAL_QUEUE_FRAMES: usize = 8192;

/// Unsynced frames that make the syncer start an fsync under `batched`.
const BATCH_FRAMES: u64 = 100;

/// Age of the oldest unsynced frame that makes the syncer start an fsync
/// under `batched`.
const BATCH_AGE: Duration = Duration::from_millis(10);

/// How long the writer may make no progress while submissions wait before
/// the process stops.
const NO_PROGRESS_BOUND: Duration = Duration::from_secs(30);

/// Bytes of an active stream segment that make the writer rotate it.
pub(super) const SEAL_BYTES: u64 = 8 * 1024 * 1024;

/// Age of the first frame of the active stream segment that makes the
/// collection tick ask the writer to rotate it. The tick runs every 5 s, so a
/// segment is rotated between 30 and 35 s after its first frame.
pub(super) const SEAL_AGE: Duration = Duration::from_secs(30);

/// Suffix of the temporary file a stream segment is written under until the
/// syncer renames it into place.
pub(super) const STREAM_TMP_SUFFIX: &str = ".tmp";

pub(crate) const WAL_FSYNCS_COUNTER: &str = "topgun_wal_fsyncs_total";
pub(crate) const WAL_FSYNC_SECONDS: &str = "topgun_wal_fsync_seconds";
pub(crate) const WAL_GROUP_FRAMES: &str = "topgun_wal_group_frames";
pub(crate) const WAL_QUEUE_DEPTH_GAUGE: &str = "topgun_wal_queue_depth";
pub(crate) const WAL_QUEUE_HEAD_AGE_GAUGE: &str = "topgun_wal_queue_head_age_seconds";
pub(crate) const WAL_UNSYNCED_FRAMES_GAUGE: &str = "topgun_wal_unsynced_frames";

/// Registers the log writer's metrics and renders each at zero, so every
/// series is present from the first scrape.
pub(crate) fn describe_log_metrics() {
    metrics::describe_counter!(
        WAL_FSYNCS_COUNTER,
        "fsyncs of the WAL stream, by the syncer"
    );
    metrics::describe_histogram!(
        WAL_FSYNC_SECONDS,
        metrics::Unit::Seconds,
        "How long one fsync of the WAL stream took"
    );
    metrics::describe_histogram!(
        WAL_GROUP_FRAMES,
        "Frames one fsync of the WAL stream made durable"
    );
    metrics::describe_gauge!(
        WAL_QUEUE_DEPTH_GAUGE,
        "Submissions waiting for the WAL writer"
    );
    metrics::describe_gauge!(
        WAL_QUEUE_HEAD_AGE_GAUGE,
        "How long the oldest submission the WAL writer took last had waited"
    );
    metrics::describe_gauge!(
        WAL_UNSYNCED_FRAMES_GAUGE,
        "Frames written to the WAL stream and not yet fsynced"
    );
    metrics::counter!(WAL_FSYNCS_COUNTER).increment(0);
    metrics::gauge!(WAL_QUEUE_DEPTH_GAUGE).set(0.0);
    metrics::gauge!(WAL_QUEUE_HEAD_AGE_GAUGE).set(0.0);
    metrics::gauge!(WAL_UNSYNCED_FRAMES_GAUGE).set(0.0);
    let _ = metrics::histogram!(WAL_FSYNC_SECONDS);
    let _ = metrics::histogram!(WAL_GROUP_FRAMES);
}

/// A position in the stream: a byte offset in stream segment `stream_seq`.
/// Ordered by segment, then offset, as the stream is written.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct DurablePosition {
    pub(crate) stream_seq: u64,
    pub(crate) offset: u64,
}

/// One append on its way to the writer: the frame's address and its encoded
/// payload, and the sender that resolves the append.
pub(crate) struct Submission {
    pub(crate) partition: u32,
    pub(crate) sequence: u64,
    pub(crate) payload: Vec<u8>,
    pub(crate) ack: oneshot::Sender<()>,
    pub(crate) enqueued: Instant,
}

/// What the writer takes from its channel, in order.
pub(crate) enum Command {
    Append(Submission),
    /// Rotate the active segment when it holds a frame at least
    /// [`SEAL_AGE`] older than `now`; `done` gets the rotated segment.
    Rotate {
        now: Instant,
        done: oneshot::Sender<Option<u64>>,
    },
    /// Resolve `done` once everything written so far is durable.
    Sync {
        done: oneshot::Sender<()>,
    },
    /// Refuse every later append, and seal the active segment: resolve
    /// `done` once it is fsynced to its end and counts as sealed.
    Close {
        done: oneshot::Sender<()>,
    },
    /// Mark the active segment sealed, as no production path can, so the
    /// next frame meets the writer's tier (P) check.
    #[cfg(test)]
    SealActive,
}

/// The syncer's part of a rotation: fsync `old` to its end, then make `new`
/// exist under its name, in that order.
struct Rotation {
    old: Option<OldSegment>,
    new: Option<NewSegment>,
    /// Resolved once the rotation is handled (a close).
    done: Option<oneshot::Sender<()>>,
}

/// A segment the writer has finished writing.
struct OldSegment {
    stream_seq: u64,
    path: PathBuf,
    header: StreamSegmentHeader,
    summary: SegmentSummary,
    /// Its final offset.
    end: u64,
    /// Frames written to the stream up to its end.
    frames_end: u64,
}

/// A segment the writer has created under its temporary name.
struct NewSegment {
    stream_seq: u64,
    tmp: PathBuf,
    path: PathBuf,
    /// The syncer's handle of the file.
    file: std::fs::File,
}

/// State the writer and the syncer share. Neither holds the lock across an
/// I/O call, and no producer takes it.
#[derive(Default)]
struct SyncState {
    /// Where the writer has written up to.
    written: Option<DurablePosition>,
    /// Where the syncer has fsynced up to.
    durable: Option<DurablePosition>,
    /// The newest segment published open.
    open: Option<u64>,
    frames_written: u64,
    frames_synced: u64,
    /// `(frames written after the group, when it was written)` of every
    /// group with an unsynced frame, oldest first.
    groups: VecDeque<(u64, Instant)>,
    /// Appends and syncs that resolve once the durable position reaches
    /// theirs, in stream order.
    waiters: VecDeque<(DurablePosition, oneshot::Sender<()>)>,
    /// Appends that resolve once their segment is published open.
    pending_open: VecDeque<(u64, oneshot::Sender<()>)>,
    rotations: VecDeque<Rotation>,
    /// The writer is gone: the syncer ends.
    stopped: bool,
}

/// What the writer and the syncer share.
pub(super) struct Shared {
    policy: WalFsyncPolicy,
    wal_dir: PathBuf,
    state: Mutex<SyncState>,
    wake: Condvar,
    /// The sealed stream segments, by `stream_seq`; the syncer adds a segment
    /// once it is fsynced to its end.
    sealed: Arc<Mutex<BTreeMap<u64, Arc<StreamSegment>>>>,
    /// The newest segment sealed by this process.
    sealed_through: watch::Sender<Option<u64>>,
    /// A fail-stop fired: nothing resolves from now on.
    failed: AtomicBool,
    /// The stream is being dropped: nothing more is written, created or
    /// renamed.
    stopping: AtomicBool,
    /// Commands sent and not yet taken by the writer.
    depth: AtomicU64,
    epoch: Instant,
    /// When the channel last went from empty to non-empty, and when the
    /// writer last took from it, in milliseconds since `epoch`.
    nonempty_since_ms: AtomicU64,
    last_drain_ms: AtomicU64,
    no_progress_bound_ms: AtomicU64,
    fsyncs: AtomicU64,
    #[cfg(test)]
    pub(super) hooks: TestHooks,
}

impl Shared {
    fn lock(&self) -> MutexGuard<'_, SyncState> {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// The clock group ages are read by; a test can drive it.
    #[cfg_attr(not(test), allow(clippy::unused_self))]
    fn now(&self) -> Instant {
        #[cfg(test)]
        if let Some(now) = *self
            .hooks
            .clock
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
        {
            return now;
        }
        Instant::now()
    }

    fn millis(&self) -> u64 {
        u64::try_from(self.epoch.elapsed().as_millis()).unwrap_or(u64::MAX)
    }

    /// Stops the process. First drops every pending resolution, so under a
    /// test build — where the fail-stop panics only the thread — no append
    /// resolves after it.
    fn fail_stop(&self, tier: WalFailStopTier, ctx: &str) -> ! {
        self.failed.store(true, Ordering::Release);
        {
            let mut state = self.lock();
            state.waiters.clear();
            state.pending_open.clear();
            state.rotations.clear();
        }
        self.wake.notify_all();
        wal_fail_stop(tier, ctx)
    }

    /// fsyncs `file`'s data, the one way the stream's files are fsynced, so a
    /// test sees each fsync and the thread it ran on.
    fn fsync(&self, file: &std::fs::File, kind: SyncKind, stream_seq: u64, offset: u64) {
        #[cfg(test)]
        self.hooks.before_fsync(kind, stream_seq, offset);
        let started = Instant::now();
        let result = file.sync_data();
        #[cfg(test)]
        let result = self.hooks.injected(result);
        if let Err(e) = result {
            self.fail_stop(
                WalFailStopTier::B,
                &format!(
                    "WAL fsync failed: {kind:?} of stream segment {} up to offset {offset}: {e}",
                    segment::format_stream_filename(stream_seq)
                ),
            );
        }
        self.fsyncs.fetch_add(1, Ordering::AcqRel);
        metrics::counter!(WAL_FSYNCS_COUNTER).increment(1);
        metrics::histogram!(WAL_FSYNC_SECONDS).record(started.elapsed().as_secs_f64());
    }

    /// fsyncs the WAL directory after a rename.
    fn fsync_dir(&self, stream_seq: u64) {
        #[cfg(test)]
        self.hooks.before_fsync(SyncKind::Dir, stream_seq, 0);
        let result = fsync_dir(&self.wal_dir);
        #[cfg(test)]
        let result = if self.hooks.fail_next_dir_fsync.swap(false, Ordering::AcqRel) {
            Err(anyhow::anyhow!("injected: the WAL directory fsync failed"))
        } else {
            result
        };
        if let Err(e) = result {
            self.fail_stop(
                WalFailStopTier::B,
                &format!(
                    "WAL directory fsync failed after stream segment {} was renamed: {e:#}",
                    segment::format_stream_filename(stream_seq)
                ),
            );
        }
    }
}

/// Which fsync of the stream ran.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SyncKind {
    /// The active segment, up to the written position.
    Data,
    /// A rotated segment, to its end.
    OldTail,
    /// A new segment's header, before its rename.
    Header,
    /// The WAL directory, after a rename.
    #[cfg(test)]
    Dir,
    /// A new segment renamed into place: not an fsync, recorded so a test
    /// sees where it falls among them.
    #[cfg(test)]
    Rename,
}

/// The writer and the syncer of one WAL, and the channel into the writer.
/// Dropping it ends both threads.
pub(super) struct LogStream {
    tx: Option<mpsc::Sender<Command>>,
    shared: Arc<Shared>,
    writer: Option<JoinHandle<()>>,
    syncer: Option<JoinHandle<()>>,
}

impl LogStream {
    /// Starts the writer and the syncer of the stream in `wal_dir`. The
    /// writer creates its first segment, `next_stream_seq`, at its first
    /// append.
    ///
    /// # Errors
    ///
    /// A thread that cannot be spawned.
    pub(super) fn start(
        wal_dir: PathBuf,
        policy: WalFsyncPolicy,
        keyring: Option<Arc<Keyring>>,
        next_stream_seq: u64,
        sealed: Arc<Mutex<BTreeMap<u64, Arc<StreamSegment>>>>,
    ) -> anyhow::Result<Self> {
        describe_log_metrics();
        let (tx, rx) = mpsc::channel(WAL_QUEUE_FRAMES);
        let shared = Arc::new(Shared {
            policy,
            wal_dir,
            state: Mutex::new(SyncState::default()),
            wake: Condvar::new(),
            sealed,
            sealed_through: watch::Sender::new(None),
            failed: AtomicBool::new(false),
            stopping: AtomicBool::new(false),
            depth: AtomicU64::new(0),
            epoch: Instant::now(),
            nonempty_since_ms: AtomicU64::new(0),
            last_drain_ms: AtomicU64::new(0),
            no_progress_bound_ms: AtomicU64::new(
                u64::try_from(NO_PROGRESS_BOUND.as_millis()).unwrap_or(u64::MAX),
            ),
            fsyncs: AtomicU64::new(0),
            #[cfg(test)]
            hooks: TestHooks::default(),
        });
        let writer = LogWriter {
            shared: Arc::clone(&shared),
            rx,
            keyring,
            active: None,
            next_stream_seq,
            frames_written: 0,
            closed: false,
            #[cfg(test)]
            sealed_for_test: false,
        };
        let writer = std::thread::Builder::new()
            .name(WRITER_THREAD.to_string())
            .spawn(move || writer.run())
            .map_err(|e| anyhow::anyhow!("Cannot start the WAL writer thread: {e}"))?;
        let syncer = Syncer {
            shared: Arc::clone(&shared),
            current: None,
        };
        let syncer = std::thread::Builder::new()
            .name(SYNCER_THREAD.to_string())
            .spawn(move || syncer.run())
            .map_err(|e| anyhow::anyhow!("Cannot start the WAL syncer thread: {e}"))?;
        Ok(Self {
            tx: Some(tx),
            shared,
            writer: Some(writer),
            syncer: Some(syncer),
        })
    }

    /// Sends `command` to the writer, waiting for room in the channel.
    ///
    /// # Errors
    ///
    /// The writer is gone.
    pub(super) async fn send(&self, command: Command) -> anyhow::Result<()> {
        let tx = self
            .tx
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("the WAL writer is stopped"))?;
        if self.shared.depth.fetch_add(1, Ordering::AcqRel) == 0 {
            self.shared
                .nonempty_since_ms
                .store(self.shared.millis(), Ordering::Release);
        }
        if tx.send(command).await.is_err() {
            self.shared.depth.fetch_sub(1, Ordering::AcqRel);
            anyhow::bail!("the WAL writer is stopped");
        }
        Ok(())
    }

    /// The newest segment this process has sealed.
    pub(super) fn sealed_through(&self) -> watch::Receiver<Option<u64>> {
        self.shared.sealed_through.subscribe()
    }

    #[cfg(test)]
    pub(super) fn shared(&self) -> &Arc<Shared> {
        &self.shared
    }
}

impl Drop for LogStream {
    fn drop(&mut self) {
        // Nothing more is written, created or renamed: a WAL opened on the
        // same directory next — which may happen as soon as the last handle
        // of this one is gone, before this drop has joined its threads — never
        // meets a file of this one appearing. The writer ends once its channel
        // is closed and drained; the syncer once the writer is gone.
        self.shared.stopping.store(true, Ordering::Release);
        self.shared.lock().stopped = true;
        self.shared.wake.notify_all();
        drop(self.tx.take());
        // A test that ends while it parks either thread must not hang here.
        #[cfg(test)]
        {
            self.shared.test_park_writer(false);
            self.shared.test_park_fsyncs(false);
        }
        if let Some(writer) = self.writer.take() {
            let _ = writer.join();
        }
        self.shared.lock().stopped = true;
        self.shared.wake.notify_all();
        if let Some(syncer) = self.syncer.take() {
            let _ = syncer.join();
        }
    }
}

const WRITER_THREAD: &str = "topgun-wal-writer";
const SYNCER_THREAD: &str = "topgun-wal-syncer";

// ---------------------------------------------------------------------------
// The writer
// ---------------------------------------------------------------------------

/// The segment the writer writes into.
struct Active {
    stream_seq: u64,
    path: PathBuf,
    header: StreamSegmentHeader,
    key: Option<SegmentKey>,
    file: std::fs::File,
    /// The written length.
    end: u64,
    /// The index of the next frame: the nonce of its seal (`TG-ENC-005`).
    next_index: u64,
    summary: SegmentSummary,
    first_frame_at: Option<Instant>,
}

/// The log writer: owns the active segment, takes every submission, frames,
/// seals and writes each group with one vectored write.
pub(crate) struct LogWriter {
    shared: Arc<Shared>,
    rx: mpsc::Receiver<Command>,
    keyring: Option<Arc<Keyring>>,
    active: Option<Active>,
    next_stream_seq: u64,
    frames_written: u64,
    closed: bool,
    #[cfg(test)]
    sealed_for_test: bool,
}

impl LogWriter {
    fn run(mut self) {
        loop {
            #[cfg(test)]
            self.shared.hooks.wait_writer_released();
            let Some(first) = self.rx.blocking_recv() else {
                break;
            };
            let mut batch = vec![first];
            while batch.len() < WAL_QUEUE_FRAMES {
                match self.rx.try_recv() {
                    Ok(command) => batch.push(command),
                    Err(_) => break,
                }
            }
            let left = self
                .shared
                .depth
                .fetch_sub(batch.len() as u64, Ordering::AcqRel)
                .saturating_sub(batch.len() as u64);
            self.shared
                .last_drain_ms
                .store(self.shared.millis(), Ordering::Release);
            // Precision loss only past 2^53, far beyond the channel.
            #[allow(clippy::cast_precision_loss)]
            metrics::gauge!(WAL_QUEUE_DEPTH_GAUGE).set(left as f64);
            let head_age = batch.iter().find_map(|command| match command {
                Command::Append(submission) => Some(submission.enqueued.elapsed()),
                _ => None,
            });
            metrics::gauge!(WAL_QUEUE_HEAD_AGE_GAUGE)
                .set(head_age.unwrap_or_default().as_secs_f64());

            let mut group = Vec::with_capacity(batch.len());
            for command in batch {
                match command {
                    Command::Append(submission) => group.push(submission),
                    other => {
                        self.write_group(std::mem::take(&mut group));
                        self.handle(other);
                    }
                }
            }
            self.write_group(group);
        }
        self.shared.lock().stopped = true;
        self.shared.wake.notify_all();
    }

    fn handle(&mut self, command: Command) {
        match command {
            Command::Append(_) => unreachable!("appends are written as a group"),
            Command::Rotate { now, done } => {
                let due = self.active.as_ref().and_then(|active| {
                    active
                        .first_frame_at
                        .filter(|first| now.saturating_duration_since(*first) >= SEAL_AGE)
                        .map(|_| active.stream_seq)
                });
                if due.is_some() {
                    self.rotate(true, None);
                }
                let _ = done.send(due);
            }
            Command::Sync { done } => self.sync(done),
            Command::Close { done } => {
                self.closed = true;
                if self.active.is_some() {
                    self.rotate(false, Some(done));
                } else {
                    let _ = done.send(());
                }
            }
            #[cfg(test)]
            Command::SealActive => self.sealed_for_test = true,
        }
    }

    /// Resolves `done` once everything written so far is durable: at once
    /// when it already is, else from the syncer. With no active segment the
    /// WAL is fresh or closed, and a close fsyncs its last segment to its end.
    fn sync(&self, done: oneshot::Sender<()>) {
        let at = self.active.as_ref().map(|active| DurablePosition {
            stream_seq: active.stream_seq,
            offset: active.end,
        });
        let mut state = self.shared.lock();
        if self.shared.failed.load(Ordering::Acquire) {
            return;
        }
        match at.or(state.written) {
            Some(at) if state.durable < Some(at) => {
                state.waiters.push_back((at, done));
                drop(state);
                self.shared.wake.notify_all();
            }
            // Already durable: no fsync would come to resolve it.
            _ => {
                drop(state);
                let _ = done.send(());
            }
        }
    }

    /// The frames of `group` for the active segment, each sealed at the next
    /// index (`TG-ENC-005`) and noted in the segment's summary, and the end
    /// offset of each.
    fn frame(&mut self, group: &[Submission]) -> (Vec<Vec<u8>>, Vec<u64>) {
        let shared = &self.shared;
        let active = self.active.as_mut().expect("an active segment");
        let mut frames = Vec::with_capacity(group.len());
        let mut ends = Vec::with_capacity(group.len());
        let mut offset = active.end;
        for submission in group {
            let index = active.next_index;
            let frame = format::frame_v3(
                submission.partition,
                submission.sequence,
                &submission.payload,
                active.key.as_ref().map(|key| (key, index)),
            )
            .unwrap_or_else(|e| {
                shared.fail_stop(
                    WalFailStopTier::B,
                    &format!(
                        "WAL frame seal failed: partition={}, sequence={}: {e}",
                        submission.partition, submission.sequence
                    ),
                )
            });
            active.next_index += 1;
            let len = u32::try_from(frame.len()).expect("a frame is at most 64 MiB");
            active.summary.note(
                submission.partition,
                FrameLocation {
                    sequence: submission.sequence,
                    offset,
                    index,
                    len,
                },
            );
            #[cfg(test)]
            shared.hooks.observe(super::WalTestEvent::Framed {
                salt: active.header.salt,
                index,
                sequence: submission.sequence,
            });
            offset += u64::from(len);
            ends.push(offset);
            frames.push(frame);
        }
        (frames, ends)
    }

    /// Frames, seals and writes `group` with one vectored write, then hands
    /// each append its resolution.
    fn write_group(&mut self, group: Vec<Submission>) {
        if group.is_empty() {
            return;
        }
        if self.closed
            || self.shared.failed.load(Ordering::Acquire)
            || self.shared.stopping.load(Ordering::Acquire)
        {
            // Dropping the senders refuses the appends: a closed or dropped
            // WAL frames nothing more, and a failed one resolves nothing.
            return;
        }
        if self.active.is_none() {
            self.rotate(true, None);
        }
        #[cfg(test)]
        if self.sealed_for_test {
            let active = self.active.as_ref().expect("an active segment");
            self.shared.fail_stop(
                WalFailStopTier::P,
                &format!(
                    "append targeted a sealed segment: path={}",
                    active.path.display()
                ),
            );
        }
        let shared = Arc::clone(&self.shared);
        let (frames, ends) = self.frame(&group);
        let active = self.active.as_mut().expect("an active segment");
        let offset = ends.last().copied().unwrap_or(active.end);
        if let Err(e) = write_all_vectored(&mut active.file, &frames) {
            shared.fail_stop(
                WalFailStopTier::B,
                &format!(
                    "WAL frame append failed: stream segment {}: {e}",
                    active.path.display()
                ),
            );
        }
        active.end = offset;
        let now = shared.now();
        active.first_frame_at.get_or_insert(now);
        self.frames_written += group.len() as u64;

        let stream_seq = active.stream_seq;
        let mut resolved = Vec::new();
        {
            let mut state = shared.lock();
            // A fail-stop sets the flag before it clears what is pending, so
            // under the lock a resolution queued here is never left behind.
            if shared.failed.load(Ordering::Acquire) {
                return;
            }
            state.written = Some(DurablePosition { stream_seq, offset });
            state.frames_written = self.frames_written;
            state.groups.push_back((self.frames_written, now));
            let open = state.open == Some(stream_seq);
            for (submission, end) in group.into_iter().zip(ends) {
                if shared.policy == WalFsyncPolicy::PerOp {
                    let at = DurablePosition {
                        stream_seq,
                        offset: end,
                    };
                    state.waiters.push_back((at, submission.ack));
                } else if open {
                    resolved.push(submission.ack);
                } else {
                    state.pending_open.push_back((stream_seq, submission.ack));
                }
            }
            set_unsynced_gauge(&state);
        }
        shared.wake.notify_all();
        for ack in resolved {
            let _ = ack.send(());
        }
        if offset >= SEAL_BYTES {
            self.rotate(true, None);
        }
    }

    /// Ends the active segment and, with `open_new`, starts the next one:
    /// writes its header into its temporary file and hands the syncer the
    /// rotation, without waiting for it (R6.4).
    fn rotate(&mut self, open_new: bool, done: Option<oneshot::Sender<()>>) {
        let old = self.active.take().map(|active| OldSegment {
            stream_seq: active.stream_seq,
            path: active.path,
            header: active.header,
            summary: active.summary,
            end: active.end,
            frames_end: self.frames_written,
        });
        let new = open_new.then(|| self.create_next());
        let written = new.as_ref().map(|new| DurablePosition {
            stream_seq: new.stream_seq,
            offset: STREAM_HEADER_LEN as u64,
        });
        {
            let mut state = self.shared.lock();
            if self.shared.failed.load(Ordering::Acquire) {
                return;
            }
            if let Some(written) = written {
                state.written = Some(written);
            }
            state.rotations.push_back(Rotation { old, new, done });
        }
        self.shared.wake.notify_all();
    }

    /// Creates the next segment's temporary file holding its header, and
    /// installs it as the active segment.
    fn create_next(&mut self) -> NewSegment {
        let stream_seq = self.next_stream_seq;
        self.next_stream_seq += 1;
        let mut header = StreamSegmentHeader::new(stream_seq);
        let key = self
            .keyring
            .as_deref()
            .and_then(|keyring| keyring.active_id().map(|id| (keyring, id)))
            .map(|(keyring, id)| {
                header.cipher = SegmentCipher::XChaCha20Poly1305;
                header.key_id = id;
                keyring.stream_segment_key(&header).unwrap_or_else(|e| {
                    self.shared.fail_stop(
                        WalFailStopTier::P,
                        &format!("stream segment {stream_seq} is sealed with {e}"),
                    )
                })
            });
        let path = self
            .shared
            .wal_dir
            .join(segment::format_stream_filename(stream_seq));
        let tmp = tmp_path(&path);
        let created = std::fs::File::create(&tmp)
            .and_then(|mut file| file.write_all(&header.encode()).map(|()| file))
            .and_then(|file| file.try_clone().map(|clone| (file, clone)));
        let (file, syncer_file) = created.unwrap_or_else(|e| {
            self.shared.fail_stop(
                WalFailStopTier::B,
                &format!("Cannot create WAL stream segment {}: {e}", tmp.display()),
            )
        });
        #[cfg(test)]
        self.shared.hooks.observe(super::WalTestEvent::Created {
            path: path.clone(),
            salt: header.salt,
        });
        self.active = Some(Active {
            stream_seq,
            path: path.clone(),
            header,
            key,
            file,
            end: STREAM_HEADER_LEN as u64,
            next_index: 0,
            summary: SegmentSummary::default(),
            first_frame_at: None,
        });
        NewSegment {
            stream_seq,
            tmp,
            path,
            file: syncer_file,
        }
    }
}

/// Waits for a resolution the writer or the syncer sends from its own thread.
///
/// Under a test runtime with a paused clock, time jumps ahead whenever no task
/// can run, and a wait on another OS thread is invisible to it: timers would
/// fire that the WAL's I/O, as blocking-pool work, used to hold back. A test
/// build therefore waits on the blocking pool, which the paused clock sees as
/// work in flight; the result is the same.
pub(super) async fn resolution<T: Send + 'static>(
    rx: oneshot::Receiver<T>,
) -> Result<T, oneshot::error::RecvError> {
    #[cfg(test)]
    return tokio::task::spawn_blocking(move || rx.blocking_recv())
        .await
        .unwrap_or_else(|e| std::panic::resume_unwind(e.into_panic()));
    #[cfg(not(test))]
    rx.await
}

/// The temporary name `path` is written under until its rename.
pub(super) fn tmp_path(path: &Path) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(STREAM_TMP_SUFFIX);
    PathBuf::from(name)
}

/// Writes every buffer of `frames`, in order, with as few `write(2)` calls as
/// the kernel allows — one for a group it takes whole.
fn write_all_vectored(file: &mut std::fs::File, frames: &[Vec<u8>]) -> std::io::Result<()> {
    let mut slices: Vec<IoSlice<'_>> = frames.iter().map(|f| IoSlice::new(f)).collect();
    let mut slices = &mut slices[..];
    while !slices.is_empty() {
        match file.write_vectored(slices) {
            Ok(0) => {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::WriteZero,
                    "the WAL segment took no byte",
                ));
            }
            Ok(n) => IoSlice::advance_slices(&mut slices, n),
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
            Err(e) => return Err(e),
        }
    }
    Ok(())
}

fn set_unsynced_gauge(state: &SyncState) {
    // Precision loss only past 2^53 frames.
    #[allow(clippy::cast_precision_loss)]
    metrics::gauge!(WAL_UNSYNCED_FRAMES_GAUGE)
        .set(state.frames_written.saturating_sub(state.frames_synced) as f64);
}

// ---------------------------------------------------------------------------
// The syncer
// ---------------------------------------------------------------------------

/// What the syncer does next, decided under the lock and done outside it.
enum Action {
    Rotate(Rotation),
    /// fsync the current segment up to `target`, covering the first
    /// `frames` frames of the stream.
    Fsync {
        target: DurablePosition,
        frames: u64,
    },
}

/// The syncer: fsyncs the stream outside the writer, publishes how far it is
/// durable, and makes rotated segments exist in order.
pub(crate) struct Syncer {
    shared: Arc<Shared>,
    /// The segment its fsyncs target: the oldest the writer has not finished
    /// handing over.
    current: Option<(u64, std::fs::File)>,
}

impl Syncer {
    fn run(mut self) {
        loop {
            let Some(action) = self.next_action() else {
                return;
            };
            match action {
                Action::Rotate(rotation) => self.rotate(rotation),
                Action::Fsync { target, frames } => {
                    let (stream_seq, file) = self.current.as_ref().expect("a current segment");
                    self.shared
                        .fsync(file, SyncKind::Data, *stream_seq, target.offset);
                    #[cfg(test)]
                    self.shared.hooks.observe_fsync(
                        &self
                            .shared
                            .wal_dir
                            .join(segment::format_stream_filename(target.stream_seq)),
                        target.offset,
                    );
                    self.publish_durable(target, frames);
                }
            }
        }
    }

    /// Waits until there is a rotation to handle or an fsync to start; `None`
    /// once the writer is gone.
    fn next_action(&self) -> Option<Action> {
        let mut state = self.shared.lock();
        loop {
            if state.stopped {
                return None;
            }
            if let Some(rotation) = state.rotations.pop_front() {
                return Some(Action::Rotate(rotation));
            }
            let now = self.shared.now();
            if let Some(action) = self.fsync_due(&state, now) {
                return Some(action);
            }
            if let Some(bound) = self.stalled() {
                drop(state);
                self.shared.fail_stop(
                    WalFailStopTier::B,
                    &format!(
                        "WAL writer made no progress for {:?}",
                        Duration::from_millis(bound)
                    ),
                );
            }
            let timeout = self.wait_bound(&state, now);
            state = self
                .shared
                .wake
                .wait_timeout(state, timeout)
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .0;
        }
    }

    /// The fsync to start now, if one is due: something is written past the
    /// durable position and an append or a sync waits for it, or, under
    /// `batched`, at least [`BATCH_FRAMES`] frames are unsynced or the oldest
    /// is at least [`BATCH_AGE`] old. The target is the written position,
    /// read here, before the call.
    fn fsync_due(&self, state: &SyncState, now: Instant) -> Option<Action> {
        let (current, _) = self.current.as_ref()?;
        let written = state.written?;
        if written.stream_seq != *current || state.durable >= Some(written) {
            return None;
        }
        let waited = !state.waiters.is_empty();
        let batched = self.shared.policy == WalFsyncPolicy::Batched && {
            #[cfg(test)]
            let held = self.shared.hooks.hold_batched.load(Ordering::Acquire);
            #[cfg(not(test))]
            let held = false;
            !held
                && (state.frames_written.saturating_sub(state.frames_synced) >= BATCH_FRAMES
                    || oldest_unsynced(state)
                        .is_some_and(|at| now.saturating_duration_since(at) >= BATCH_AGE))
        };
        (waited || batched).then_some(Action::Fsync {
            target: written,
            frames: state.frames_written,
        })
    }

    /// How long to wait before looking again when nothing is due: until the
    /// oldest unsynced frame is [`BATCH_AGE`] old, and at most a quarter of
    /// the writer's progress bound, so a stalled writer is seen in time.
    fn wait_bound(&self, state: &SyncState, now: Instant) -> Duration {
        let bound =
            Duration::from_millis(self.shared.no_progress_bound_ms.load(Ordering::Acquire) / 4)
                .clamp(Duration::from_millis(1), Duration::from_secs(1));
        match oldest_unsynced(state) {
            Some(at) if self.shared.policy == WalFsyncPolicy::Batched => {
                (at + BATCH_AGE).saturating_duration_since(now).min(bound)
            }
            _ => bound,
        }
    }

    /// The writer's progress bound in milliseconds, when submissions have
    /// waited longer than it with no progress of the writer.
    fn stalled(&self) -> Option<u64> {
        let shared = &self.shared;
        if shared.depth.load(Ordering::Acquire) == 0 {
            return None;
        }
        let since = shared
            .nonempty_since_ms
            .load(Ordering::Acquire)
            .max(shared.last_drain_ms.load(Ordering::Acquire));
        let bound = shared.no_progress_bound_ms.load(Ordering::Acquire);
        (shared.millis().saturating_sub(since) >= bound).then_some(bound)
    }

    /// Publishes `target` durable and resolves every waiter at or below it.
    fn publish_durable(&self, target: DurablePosition, frames: u64) {
        let mut resolved = Vec::new();
        {
            let mut state = self.shared.lock();
            let before = state.frames_synced;
            state.durable = state.durable.max(Some(target));
            state.frames_synced = state.frames_synced.max(frames);
            let synced = state.frames_synced;
            while state.groups.front().is_some_and(|(end, _)| *end <= synced) {
                state.groups.pop_front();
            }
            while state
                .waiters
                .front()
                .is_some_and(|(at, _)| Some(*at) <= state.durable)
            {
                resolved.push(state.waiters.pop_front().expect("checked").1);
            }
            // Precision loss only past 2^53 frames.
            #[allow(clippy::cast_precision_loss)]
            metrics::histogram!(WAL_GROUP_FRAMES).record(synced.saturating_sub(before) as f64);
            set_unsynced_gauge(&state);
        }
        for ack in resolved {
            let _ = ack.send(());
        }
    }

    /// Handles one rotation, in this order: fsync the old segment to its end
    /// and publish that durable → fsync the new segment's header → rename it
    /// into place → fsync the directory → publish it open. Under `none`, only
    /// the rename and the publication (no fsync), unless a sync waits or the
    /// rotation is a close.
    fn rotate(&mut self, rotation: Rotation) {
        let shared = Arc::clone(&self.shared);
        let fsyncs = shared.policy != WalFsyncPolicy::None;
        if let Some(old) = rotation.old {
            // Under `none`, a sync that waits makes the old tail durable too:
            // a sync covers everything written before it.
            let waited = !shared.lock().waiters.is_empty();
            if fsyncs || waited || rotation.done.is_some() {
                let (_, file) = self.current.as_ref().expect("the old segment's handle");
                shared.fsync(file, SyncKind::OldTail, old.stream_seq, old.end);
                #[cfg(test)]
                shared.hooks.observe_fsync(&old.path, old.end);
                self.publish_durable(
                    DurablePosition {
                        stream_seq: old.stream_seq,
                        offset: old.end,
                    },
                    old.frames_end,
                );
            }
            self.current = None;
            let mut summary = old.summary;
            for locations in summary.frames.values_mut() {
                locations.shrink_to_fit();
            }
            shared
                .sealed
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .insert(
                    old.stream_seq,
                    Arc::new(StreamSegment {
                        path: old.path,
                        header: Some(old.header),
                        summary,
                    }),
                );
            shared.sealed_through.send_replace(Some(old.stream_seq));
            #[cfg(test)]
            shared.hooks.crash_image();
        }
        if let Some(new) = rotation.new {
            if fsyncs {
                shared.fsync(
                    &new.file,
                    SyncKind::Header,
                    new.stream_seq,
                    STREAM_HEADER_LEN as u64,
                );
                // Durable under the name it is renamed to next.
                #[cfg(test)]
                shared
                    .hooks
                    .observe_fsync(&new.path, STREAM_HEADER_LEN as u64);
            }
            #[cfg(test)]
            super::test_abort_if_armed(super::TestAbortPoint::CreateBeforeRename);
            if shared.stopping.load(Ordering::Acquire) {
                // Abandoned with the stream: its frames resolved nothing.
                return;
            }
            if let Err(e) = std::fs::rename(&new.tmp, &new.path) {
                shared.fail_stop(
                    WalFailStopTier::B,
                    &format!(
                        "Cannot rename {} to {}: {e}",
                        new.tmp.display(),
                        new.path.display()
                    ),
                );
            }
            #[cfg(test)]
            shared.hooks.record(SyncKind::Rename, new.stream_seq, 0);
            if fsyncs {
                shared.fsync_dir(new.stream_seq);
            }
            self.current = Some((new.stream_seq, new.file));
            let mut resolved = Vec::new();
            {
                let mut state = shared.lock();
                state.open = Some(new.stream_seq);
                while state
                    .pending_open
                    .front()
                    .is_some_and(|(seq, _)| *seq <= new.stream_seq)
                {
                    resolved.push(state.pending_open.pop_front().expect("checked").1);
                }
            }
            for ack in resolved {
                let _ = ack.send(());
            }
        }
        if let Some(done) = rotation.done {
            let _ = done.send(());
        }
    }
}

/// When the oldest unsynced frame was written.
fn oldest_unsynced(state: &SyncState) -> Option<Instant> {
    state
        .groups
        .iter()
        .find(|(end, _)| *end > state.frames_synced)
        .map(|(_, at)| *at)
}

// ---------------------------------------------------------------------------
// Test hooks
// ---------------------------------------------------------------------------

/// One fsync or rename the syncer (or, in a broken build, anything else) ran.
#[cfg(test)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SyncEvent {
    pub(crate) kind: SyncKind,
    pub(crate) stream_seq: u64,
    pub(crate) offset: u64,
    /// The name of the thread it ran on.
    pub(crate) thread: Option<String>,
}

#[cfg(test)]
#[derive(Default)]
struct Gates {
    /// While set, every fsync waits before its call, unless `pass` lets it.
    park_fsyncs: bool,
    /// Parked fsyncs still let through.
    pass: usize,
    /// fsyncs waiting at the gate.
    parked_fsyncs: usize,
    /// While set, the writer takes nothing from its channel.
    park_writer: bool,
    writer_parked: bool,
}

#[cfg(test)]
#[derive(Default)]
#[allow(clippy::type_complexity)]
pub(super) struct TestHooks {
    gates: Mutex<Gates>,
    gates_changed: Condvar,
    fail_next_fsync: AtomicBool,
    pub(super) fail_next_dir_fsync: AtomicBool,
    pub(super) hold_batched: AtomicBool,
    pub(super) clock: Mutex<Option<Instant>>,
    events: Mutex<Vec<SyncEvent>>,
    pub(super) fsync_observer: Mutex<Option<Arc<dyn Fn(&Path, u64) + Send + Sync>>>,
    pub(super) observer: Mutex<Option<Arc<dyn Fn(super::WalTestEvent) + Send + Sync>>>,
    pub(super) crash_image: Mutex<Option<Arc<dyn Fn() + Send + Sync>>>,
}

#[cfg(test)]
impl TestHooks {
    fn gates(&self) -> MutexGuard<'_, Gates> {
        self.gates
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn record(&self, kind: SyncKind, stream_seq: u64, offset: u64) {
        self.events
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(SyncEvent {
                kind,
                stream_seq,
                offset,
                thread: std::thread::current().name().map(str::to_string),
            });
    }

    fn before_fsync(&self, kind: SyncKind, stream_seq: u64, offset: u64) {
        let mut gates = self.gates();
        if gates.park_fsyncs {
            gates.parked_fsyncs += 1;
            self.gates_changed.notify_all();
            while gates.park_fsyncs && gates.pass == 0 {
                gates = self
                    .gates_changed
                    .wait(gates)
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
            }
            if gates.park_fsyncs {
                gates.pass -= 1;
            }
            gates.parked_fsyncs -= 1;
            self.gates_changed.notify_all();
        }
        drop(gates);
        self.record(kind, stream_seq, offset);
    }

    fn injected(&self, result: std::io::Result<()>) -> std::io::Result<()> {
        if self.fail_next_fsync.swap(false, Ordering::AcqRel) {
            return Err(std::io::Error::other("injected: the fsync failed"));
        }
        result
    }

    fn wait_writer_released(&self) {
        let mut gates = self.gates();
        while gates.park_writer {
            gates.writer_parked = true;
            self.gates_changed.notify_all();
            gates = self
                .gates_changed
                .wait(gates)
                .unwrap_or_else(std::sync::PoisonError::into_inner);
        }
        gates.writer_parked = false;
    }

    fn observe(&self, event: super::WalTestEvent) {
        let observer = self
            .observer
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        if let Some(observer) = observer {
            observer(event);
        }
    }

    fn observe_fsync(&self, path: &Path, length: u64) {
        let observer = self
            .fsync_observer
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        if let Some(observer) = observer {
            observer(path, length);
        }
    }

    fn crash_image(&self) {
        let hook = self
            .crash_image
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        if let Some(hook) = hook {
            hook();
        }
    }
}

/// Test controls of the writer and the syncer.
#[cfg(test)]
impl Shared {
    /// Parks every fsync before its call while `park` is set.
    pub(super) fn test_park_fsyncs(&self, park: bool) {
        self.hooks.gates().park_fsyncs = park;
        self.hooks.gates_changed.notify_all();
    }

    /// Lets `n` more parked fsyncs through.
    pub(super) fn test_pass_fsyncs(&self, n: usize) {
        self.hooks.gates().pass += n;
        self.hooks.gates_changed.notify_all();
    }

    /// Frames the writer has written.
    pub(super) fn test_frames_written(&self) -> u64 {
        self.lock().frames_written
    }

    /// Waits until `n` fsyncs are parked.
    pub(super) fn test_wait_parked_fsyncs(&self, n: usize) {
        let mut gates = self.hooks.gates();
        while gates.parked_fsyncs < n {
            gates = self
                .hooks
                .gates_changed
                .wait_timeout(gates, Duration::from_secs(30))
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .0;
        }
    }

    /// Parks the writer before it next takes from its channel while `park`
    /// is set.
    pub(super) fn test_park_writer(&self, park: bool) {
        self.hooks.gates().park_writer = park;
        self.hooks.gates_changed.notify_all();
    }

    /// Whether the writer is parked.
    pub(super) fn test_writer_parked(&self) -> bool {
        self.hooks.gates().writer_parked
    }

    /// Makes the next fsync of a stream file fail.
    pub(super) fn test_fail_next_fsync(&self) {
        self.hooks.fail_next_fsync.store(true, Ordering::Release);
    }

    /// Every fsync and rename so far, oldest first.
    pub(super) fn test_events(&self) -> Vec<SyncEvent> {
        self.hooks
            .events
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    /// fsyncs of the stream so far, the directory's not included.
    pub(super) fn test_fsyncs(&self) -> u64 {
        self.fsyncs.load(Ordering::Acquire)
    }

    /// The position the writer has written up to.
    pub(super) fn test_written(&self) -> Option<DurablePosition> {
        self.lock().written
    }

    /// The position the syncer has published durable.
    pub(super) fn test_durable(&self) -> Option<DurablePosition> {
        self.lock().durable
    }

    /// Shortens the writer's progress bound.
    pub(super) fn test_set_no_progress_bound(&self, bound: Duration) {
        self.no_progress_bound_ms.store(
            u64::try_from(bound.as_millis()).unwrap_or(u64::MAX),
            Ordering::Release,
        );
        self.wake.notify_all();
    }

    /// Wakes the syncer to look at the clock again.
    pub(super) fn test_wake(&self) {
        self.wake.notify_all();
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use std::path::Path;
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    use topgun_core::hlc::Timestamp;
    use topgun_core::types::Value;

    use super::{DurablePosition, Shared, SyncKind, BATCH_AGE, WRITER_THREAD};
    use crate::storage::datastores::NullDataStore;
    use crate::storage::map_data_store::{MapDataStore, WatermarkKeeping};
    use crate::storage::record::RecordValue;
    use crate::storage::wal::segment::{self, inspect};
    use crate::storage::wal::{
        Wal, WalEntry, WalFailStopTier, WalFsyncPolicy, WalOp, WalStorePayload, WalWriter,
        FAIL_STOP_TEST_LOCK,
    };

    fn open(dir: &Path, policy: WalFsyncPolicy) -> Arc<WalWriter> {
        let store: Arc<dyn MapDataStore> = Arc::new(WatermarkKeeping::new(NullDataStore));
        let wal = WalWriter::open_over_store(dir.to_path_buf(), policy, Arc::clone(&store))
            .expect("open the WAL");
        // The WAL holds the store weakly; the test keeps it for the WAL's life.
        std::mem::forget(store);
        wal
    }

    fn entry(sequence: u64) -> WalEntry {
        let timestamp = Timestamp {
            millis: sequence,
            counter: 0,
            node_id: "n".to_string(),
        };
        WalEntry {
            map: "m".to_string(),
            key: format!("k{sequence}"),
            op: WalOp::Store {
                value: WalStorePayload::Record(RecordValue::Lww {
                    value: Value::String("v".to_string()),
                    timestamp: timestamp.clone(),
                    ttl_ms: None,
                }),
            },
            timestamp: Some(timestamp),
            sequence,
        }
    }

    fn spawn_append(
        wal: &Arc<WalWriter>,
        partition: u32,
        sequence: u64,
    ) -> tokio::task::JoinHandle<anyhow::Result<()>> {
        let wal = Arc::clone(wal);
        tokio::spawn(async move { wal.append(partition, &entry(sequence)).await })
    }

    /// Polls `done` until it holds, failing after 10 s.
    async fn until(what: &str, mut done: impl FnMut() -> bool) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while !done() {
            assert!(Instant::now() < deadline, "timed out waiting for {what}");
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
    }

    /// Lets `ms` of real time pass, for a negative check.
    async fn quiet(ms: u64) {
        tokio::time::sleep(Duration::from_millis(ms)).await;
    }

    fn log(wal: &WalWriter) -> &Arc<Shared> {
        wal.test_log()
    }

    fn data_fsyncs(log: &Shared) -> usize {
        log.test_events()
            .iter()
            .filter(|e| e.kind == SyncKind::Data)
            .count()
    }

    fn no_fsync_on_the_writer(log: &Shared) {
        for event in log.test_events() {
            assert_ne!(
                event.thread.as_deref(),
                Some(WRITER_THREAD),
                "an fsync ran on the writer thread: {event:?}"
            );
        }
    }

    /// `AC1`: with the syncer's fsync parked, 64 `per_op` appends over 64
    /// partitions are written and none resolves; released, all resolve, made
    /// durable by at most two fsyncs.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn r_e_per_op_resolves_only_after_covering_fsync() {
        let dir = tempfile::tempdir().unwrap();
        let wal = open(dir.path(), WalFsyncPolicy::PerOp);
        wal.append(0, &entry(1)).await.unwrap();
        let log = Arc::clone(log(&wal));
        let fsyncs_before = log.test_fsyncs();

        log.test_park_fsyncs(true);
        let appends: Vec<_> = (1..=64u32)
            .map(|p| spawn_append(&wal, p, u64::from(p) + 1))
            .collect();
        until("64 frames written", || log.test_frames_written() == 65).await;
        log.test_wait_parked_fsyncs(1);
        quiet(50).await;
        assert!(
            appends.iter().all(|a| !a.is_finished()),
            "a per_op append resolved before an fsync covering it returned"
        );
        assert!(log.test_durable() < log.test_written());

        log.test_park_fsyncs(false);
        for append in appends {
            append.await.unwrap().expect("append");
        }
        let fsyncs = log.test_fsyncs() - fsyncs_before;
        assert!(fsyncs <= 2, "64 concurrent appends took {fsyncs} fsyncs");
        assert!(log.test_durable() >= log.test_written());
        no_fsync_on_the_writer(&log);
    }

    /// `AC2`: with an fsync parked, new `batched` appends still resolve: the
    /// writer does not wait for the syncer.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn r_e_writer_writes_while_fsync_runs() {
        let dir = tempfile::tempdir().unwrap();
        let wal = open(dir.path(), WalFsyncPolicy::Batched);
        wal.append(0, &entry(1)).await.unwrap();
        let log = Arc::clone(log(&wal));
        log.test_park_fsyncs(true);
        // 100 unsynced frames start an fsync, which parks.
        for sequence in 2..=101 {
            wal.append(u32::try_from(sequence % 271).unwrap(), &entry(sequence))
                .await
                .unwrap();
        }
        log.test_wait_parked_fsyncs(1);
        let written = log.test_written();
        tokio::time::timeout(Duration::from_secs(5), async {
            for sequence in 102..=300 {
                wal.append(u32::try_from(sequence % 271).unwrap(), &entry(sequence))
                    .await
                    .unwrap();
            }
        })
        .await
        .expect("batched appends resolve while an fsync is parked");
        assert!(log.test_written() > written);
        log.test_park_fsyncs(false);
        wal.sync().await.unwrap();
        no_fsync_on_the_writer(&log);
    }

    /// `AC3`: 10 000 `batched` appends over 271 partitions take at most one
    /// fsync per 10 ms window plus one per 100 frames, plus one.
    #[tokio::test(flavor = "multi_thread", worker_threads = 8)]
    async fn r_e_batched_fsync_rate_is_independent_of_partitions() {
        const APPENDS: u64 = 10_000;
        let dir = tempfile::tempdir().unwrap();
        let wal = open(dir.path(), WalFsyncPolicy::Batched);
        wal.append(0, &entry(1)).await.unwrap();
        let log = Arc::clone(log(&wal));
        let before = data_fsyncs(&log);
        let started = Instant::now();
        let tasks: Vec<_> = (0..271u32)
            .map(|p| {
                let wal = Arc::clone(&wal);
                tokio::spawn(async move {
                    let mut sequence = 2 + u64::from(p);
                    while sequence < APPENDS + 2 {
                        wal.append(p, &entry(sequence)).await.unwrap();
                        sequence += 271;
                    }
                })
            })
            .collect();
        for task in tasks {
            task.await.unwrap();
        }
        let elapsed = started.elapsed();
        let fsyncs = data_fsyncs(&log) - before;
        let windows = u64::try_from(elapsed.as_millis() / BATCH_AGE.as_millis()).unwrap();
        let bound = windows + 1 + APPENDS / 100;
        println!(
            "R_E_BATCHED appends={APPENDS} elapsed_ms={} fsyncs={fsyncs} bound={bound}",
            elapsed.as_millis()
        );
        assert!(
            fsyncs as u64 <= bound,
            "{fsyncs} fsyncs for {APPENDS} appends over 271 partitions in {elapsed:?}, bound {bound}"
        );
    }

    /// `AC4`: a rotated segment is not eligible for collection before the
    /// syncer reports it durable to its end.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn r_e_rotation_seals_only_after_sync() {
        let dir = tempfile::tempdir().unwrap();
        let wal = open(dir.path(), WalFsyncPolicy::Batched);
        for sequence in 1..=5 {
            wal.append(7, &entry(sequence)).await.unwrap();
        }
        wal.sync().await.unwrap();
        // Every frame is at or below the durable watermark: only sealing
        // stands between the segment and its collection.
        wal.mark_applied(7, 5).await.unwrap();
        let log = Arc::clone(log(&wal));
        log.test_park_fsyncs(true);
        let rotated = wal.test_rotate_now().await.expect("a rotation");
        log.test_wait_parked_fsyncs(1);
        quiet(50).await;
        let all_covered = vec![u64::MAX; topgun_core::PARTITION_COUNT as usize];
        let mut next = vec![0; topgun_core::PARTITION_COUNT as usize];
        assert!(wal.test_sealed_paths(7).await.is_empty());
        assert!(
            wal.covered_stream_segments(&all_covered, &mut next)
                .is_empty(),
            "a rotated segment is collectable before its fsync to its end"
        );
        log.test_park_fsyncs(false);
        wal.test_wait_sealed(rotated).await;
        assert_eq!(
            wal.test_sealed_paths(7).await,
            vec![dir.path().join(segment::format_stream_filename(rotated))]
        );
        assert_eq!(
            wal.covered_stream_segments(&all_covered, &mut next).len(),
            1
        );
    }

    /// `AC5`: a writer parked longer than its (shortened) progress bound while
    /// a submission waits is a tier (B) fail-stop.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn r_e_no_progress_fail_stops() {
        let _guard = FAIL_STOP_TEST_LOCK.lock().await;
        let observed = WalWriter::test_fail_stop_observations().len();
        let dir = tempfile::tempdir().unwrap();
        let wal = open(dir.path(), WalFsyncPolicy::Batched);
        let log = Arc::clone(log(&wal));
        log.test_set_no_progress_bound(Duration::from_millis(200));
        log.test_park_writer(true);
        // The writer looks at the gate between two takes: a sync brings it
        // back to the gate, or waits in the channel if it parked first.
        let sync = {
            let wal = Arc::clone(&wal);
            tokio::spawn(async move { wal.sync().await })
        };
        until("the writer parked", || log.test_writer_parked()).await;
        let append = spawn_append(&wal, 0, 1);
        until("the fail-stop", || {
            WalWriter::test_fail_stop_observations().len() > observed
        })
        .await;
        assert_eq!(
            WalWriter::test_fail_stop_observations().get(observed),
            Some(&WalFailStopTier::B)
        );
        log.test_park_writer(false);
        assert!(
            append.await.unwrap().is_err(),
            "an append resolved after a fail-stop"
        );
        let _ = sync.await;
    }

    /// `AC5a`, `batched`: with the old tail's fsync parked the writer rotates and
    /// keeps writing into the new segment, no frame of which resolves before
    /// the syncer has, in order, fsynced the old tail and the new header,
    /// renamed the new segment and fsynced the directory.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn r_e_rotation_never_waits_on_fsync() {
        let dir = tempfile::tempdir().unwrap();
        let wal = open(dir.path(), WalFsyncPolicy::Batched);
        wal.append(0, &entry(1)).await.unwrap();
        wal.sync().await.unwrap();
        let log = Arc::clone(log(&wal));
        wal.test_hold_batched_fsync(true);
        log.test_park_fsyncs(true);
        let old = wal.test_rotate_now().await.expect("a rotation");
        log.test_wait_parked_fsyncs(1);
        let events_before = log.test_events().len();
        let appends: Vec<_> = (2..=20u64)
            .map(|s| spawn_append(&wal, u32::try_from(s).unwrap(), s))
            .collect();
        until("frames written into the new segment", || {
            log.test_frames_written() == 20
        })
        .await;
        let written = log.test_written().expect("a written position");
        assert_eq!(
            written.stream_seq,
            old + 1,
            "the writer writes the new segment"
        );
        quiet(50).await;
        assert!(
            appends.iter().all(|a| !a.is_finished()),
            "a frame of the new segment resolved before it was published open"
        );
        assert_eq!(
            log.test_events().len(),
            events_before,
            "nothing ran past the park"
        );

        log.test_park_fsyncs(false);
        for append in appends {
            append.await.unwrap().expect("append");
        }
        let order: Vec<(SyncKind, u64)> = log.test_events()[events_before..]
            .iter()
            .map(|e| (e.kind, e.stream_seq))
            .take(4)
            .collect();
        assert_eq!(
            order,
            vec![
                (SyncKind::OldTail, old),
                (SyncKind::Header, old + 1),
                (SyncKind::Rename, old + 1),
                (SyncKind::Dir, old + 1),
            ]
        );
        no_fsync_on_the_writer(&log);
        wal.test_hold_batched_fsync(false);
    }

    /// `AC5a`, `per_op`: a frame written into the old segment after its last
    /// regular fsync resolves exactly when the old tail's fsync returns,
    /// and the durable position is then the old segment's end.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn r_e_rotation_old_tail_fsync_resolves_per_op() {
        let dir = tempfile::tempdir().unwrap();
        let wal = open(dir.path(), WalFsyncPolicy::PerOp);
        wal.append(0, &entry(1)).await.unwrap();
        let log = Arc::clone(log(&wal));
        log.test_park_fsyncs(true);
        let first = spawn_append(&wal, 1, 2);
        log.test_wait_parked_fsyncs(1);
        let tail = spawn_append(&wal, 2, 3);
        until("the tail frame written", || log.test_frames_written() == 3).await;
        let end = log.test_written().expect("written").offset;
        let old = wal.test_rotate_now().await.expect("a rotation");

        // The regular fsync parked before the tail frame was written: it
        // resolves the first frame and not the tail.
        log.test_pass_fsyncs(1);
        first.await.unwrap().expect("first");
        log.test_wait_parked_fsyncs(1);
        quiet(50).await;
        assert!(
            !tail.is_finished(),
            "the tail resolved before the old tail's fsync"
        );
        assert_eq!(
            log.test_events().last().map(|e| e.kind),
            Some(SyncKind::Data),
            "the syncer is parked at the old tail's fsync"
        );

        // The old tail's fsync returns; the syncer parks at the new header.
        log.test_pass_fsyncs(1);
        tokio::time::timeout(Duration::from_secs(5), tail)
            .await
            .expect("the tail resolves when the old tail's fsync returns")
            .unwrap()
            .expect("tail");
        assert_eq!(
            log.test_durable(),
            Some(DurablePosition {
                stream_seq: old,
                offset: end
            })
        );
        log.test_park_fsyncs(false);
    }

    /// `AC5a`, `none`: a rotation runs no fsync of any kind, and the new
    /// segment's frames resolve once its rename is published open.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn r_e_rotation_under_none_runs_no_fsync() {
        let dir = tempfile::tempdir().unwrap();
        let wal = open(dir.path(), WalFsyncPolicy::None);
        wal.append(0, &entry(1)).await.unwrap();
        let log = Arc::clone(log(&wal));
        let old = wal.test_rotate_now().await.expect("a rotation");
        for sequence in 2..=10 {
            wal.append(1, &entry(sequence)).await.unwrap();
        }
        wal.test_wait_sealed(old).await;
        assert_eq!(log.test_fsyncs(), 0, "an fsync ran under none");
        let kinds: Vec<SyncKind> = log.test_events().iter().map(|e| e.kind).collect();
        assert_eq!(kinds, vec![SyncKind::Rename, SyncKind::Rename]);
    }

    /// `AC5a`, crash image: a crash between the old tail's fsync and the new
    /// header's leaves the old segment newest and the new segment's temporary
    /// file with frames in it; neither `inspect` nor recovery reads it, and
    /// the next open deletes it before recovery.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn r_e_rotation_crash_image_recovers_the_old_segment_newest() {
        let dir = tempfile::tempdir().unwrap();
        let image = tempfile::tempdir().unwrap();
        let wal = open(dir.path(), WalFsyncPolicy::Batched);
        for sequence in 1..=3 {
            wal.append(5, &entry(sequence)).await.unwrap();
        }
        wal.sync().await.unwrap();
        let log = Arc::clone(log(&wal));
        wal.test_hold_batched_fsync(true);
        {
            let from = dir.path().to_path_buf();
            let to = image.path().to_path_buf();
            *log.hooks
                .crash_image
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(Arc::new(move || {
                // `wal-collected` stays behind: the image is opened over a
                // fresh store, as a data directory it belongs with.
                for entry in std::fs::read_dir(&from).unwrap() {
                    let entry = entry.unwrap();
                    if entry.file_name() != crate::storage::wal::WAL_COLLECTED_FILE {
                        std::fs::copy(entry.path(), to.join(entry.file_name())).unwrap();
                    }
                }
            }));
        }
        log.test_park_fsyncs(true);
        let old = wal.test_rotate_now().await.expect("a rotation");
        log.test_wait_parked_fsyncs(1);
        let appends: Vec<_> = (4..=8u64).map(|s| spawn_append(&wal, 6, s)).collect();
        until("frames in the new segment", || {
            log.test_frames_written() == 8
        })
        .await;
        log.test_pass_fsyncs(1);
        log.test_wait_parked_fsyncs(1);
        log.test_park_fsyncs(false);
        for append in appends {
            append.await.unwrap().unwrap();
        }
        wal.test_hold_batched_fsync(false);

        let tmp = super::tmp_path(&image.path().join(segment::format_stream_filename(old + 1)));
        assert!(
            std::fs::metadata(&tmp).unwrap().len() > segment::STREAM_HEADER_LEN as u64,
            "the image holds the new segment's temporary file with frames"
        );
        let frames = inspect::frames_on_disk(image.path()).expect("inspect the image");
        assert!(
            frames.iter().all(|f| f.partition == 5),
            "inspect read a frame of the temporary file: {frames:?}"
        );
        let reopened = open(image.path(), WalFsyncPolicy::Batched);
        assert!(!tmp.exists(), "the open kept the temporary file");
        assert!(reopened.unapplied(6).await.unwrap().is_empty());
        assert_eq!(
            reopened
                .unapplied(5)
                .await
                .unwrap()
                .iter()
                .map(|e| e.sequence)
                .collect::<Vec<_>>(),
            vec![1, 2, 3]
        );
        assert_eq!(
            inspect::segment_paths(image.path()),
            vec![image.path().join(segment::format_stream_filename(old))]
        );
    }

    /// `AC6a`: a failed fsync of the syncer is a tier (B) fail-stop under
    /// `batched` and under `per_op`, and no append resolves `Ok` after it.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn r_e_batched_fsync_error_fail_stops() {
        let _guard = FAIL_STOP_TEST_LOCK.lock().await;
        for policy in [WalFsyncPolicy::Batched, WalFsyncPolicy::PerOp] {
            let observed = WalWriter::test_fail_stop_observations().len();
            let dir = tempfile::tempdir().unwrap();
            let wal = open(dir.path(), policy);
            wal.append(0, &entry(1)).await.unwrap();
            wal.sync().await.unwrap();
            let log = Arc::clone(log(&wal));
            log.test_fail_next_fsync();
            // Enough frames to start an fsync under either policy.
            let appends: Vec<_> = (2..=150u64)
                .map(|s| spawn_append(&wal, u32::try_from(s % 271).unwrap(), s))
                .collect();
            until("the fail-stop", || {
                WalWriter::test_fail_stop_observations().len() > observed
            })
            .await;
            assert_eq!(
                WalWriter::test_fail_stop_observations().get(observed),
                Some(&WalFailStopTier::B),
                "{policy:?}"
            );
            for append in appends {
                let _ = append.await;
            }
            assert!(
                wal.append(1, &entry(1000)).await.is_err(),
                "{policy:?}: an append resolved Ok after the fail-stop"
            );
        }
    }

    /// `AC6b`: under `batched`, with the syncer's clock held, 100 unsynced
    /// frames start an fsync without the clock moving, and one unsynced frame
    /// is fsynced once it is 10 ms old and not before; its durable position
    /// is published only once that fsync returned.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn r_e_batched_window_is_bounded() {
        let dir = tempfile::tempdir().unwrap();
        let wal = open(dir.path(), WalFsyncPolicy::Batched);
        let log = Arc::clone(log(&wal));
        let t0 = Instant::now();
        let set_clock = |at: Instant| {
            *log.hooks
                .clock
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(at);
            log.test_wake();
        };
        set_clock(t0);

        // (ii) one frame: no fsync before it is 10 ms old.
        wal.append(0, &entry(1)).await.unwrap();
        quiet(100).await;
        assert_eq!(
            data_fsyncs(&log),
            0,
            "an fsync before the frame was 10 ms old"
        );
        set_clock(t0 + Duration::from_millis(9));
        quiet(100).await;
        assert_eq!(data_fsyncs(&log), 0, "an fsync at 9 ms");
        log.test_park_fsyncs(true);
        set_clock(t0 + BATCH_AGE);
        log.test_wait_parked_fsyncs(1);
        assert!(
            log.test_durable() < log.test_written(),
            "durable before the fsync returned"
        );
        log.test_park_fsyncs(false);
        until("the frame durable", || {
            log.test_durable() >= log.test_written()
        })
        .await;
        assert_eq!(data_fsyncs(&log), 1);

        // (i) 100 frames: an fsync with the clock held.
        for sequence in 2..=100 {
            wal.append(1, &entry(sequence)).await.unwrap();
        }
        quiet(100).await;
        assert_eq!(data_fsyncs(&log), 1, "an fsync below 100 unsynced frames");
        wal.append(1, &entry(101)).await.unwrap();
        until("the 100-frame fsync", || data_fsyncs(&log) == 2).await;
    }
}
