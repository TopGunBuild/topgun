//! Resolvable sink behind the OR-Map tombstone-bytes gauge.
//!
//! The gauge is a process-global monitoring signal written from every OR-remove
//! and inbound-sync union in the process. That is correct for production — there
//! is exactly one process and exactly one number to export — but it makes any
//! test that asserts on the gauge observe the whole crate's concurrent traffic,
//! not its own. This module puts a resolvable sink between the gauge's public
//! API and its storage, so a test can bind a private sink for the duration of
//! its own future and read a delta nothing else can perturb.
//!
//! Production resolves to the one process gauge through a `cfg`-split resolver, so a
//! release build carries neither a branch nor a vtable: it calls the same
//! atomics and emits the same two Prometheus series it always has.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

#[cfg(test)]
use std::future::Future;
#[cfg(test)]
use std::sync::Arc;

/// A destination for OR-Map tombstone-byte accounting.
///
/// `Send + Sync` is required because the gauge is reachable from any tokio
/// worker thread: the write path runs wherever the task happens to be scheduled.
pub trait TombstoneGaugeSink: Send + Sync {
    /// Adds `n` bytes to the gauge.
    fn add(&self, n: u64);
    /// Subtracts `n` bytes from the gauge.
    fn sub(&self, n: u64);
    /// Reads the gauge's current value.
    fn read(&self) -> u64;
    /// Re-baselines the gauge to an absolute `total`.
    fn set(&self, total: u64);
}

/// The production sink: an `AtomicU64` plus the exported Prometheus series.
///
/// Instantiable rather than a bag of free statics so a test can bind a fresh,
/// private instance and observe *its* tripwire, instead of reading a
/// process-global flag that every unmarked OR-remove in the crate arms.
/// Production reaches the gauge only through the single process instance, so
/// this changes nothing about how the process behaves.
pub struct ProcessGauge {
    /// Running total of OR-Map tombstone bytes (sum of removed tags' UTF-8 byte
    /// lengths currently tracked across all `OrMap.tombstones` sets).
    ///
    /// This is the in-process source of truth the unbounded-tombstone-growth
    /// soak monitor (and the `/metrics` Prometheus surface) reads, giving them a
    /// cheap, lock-free signal of how much tombstone data has accumulated
    /// without walking every resident `OrMap` record on each check. `Relaxed`
    /// ordering is sufficient: this is a monitoring counter, not a
    /// correctness-critical value guarding any invariant, so no other memory
    /// operation needs to be ordered against it.
    bytes: AtomicU64,

    /// Fail-loud tripwire recording whether [`ProcessGauge::add`] has fired on
    /// this instance.
    ///
    /// [`ProcessGauge::set`] pairs an **absolute** `AtomicU64` store with an
    /// **additive** Prometheus `increment`. The one-time boot seed is only
    /// correct on a fresh recorder: if any add landed between recorder install
    /// and the boot seed, the `AtomicU64` would still self-correct (it is a
    /// store), but the Prometheus counter — the sink the soak harness scrapes —
    /// would silently double-count (it is additive). No add call site is
    /// reachable before `set_ready()` today, so the boot seed is the sole gauge
    /// mutator in the recovery→ready window; this flag turns a future refactor
    /// that violates that ordering into a loud failure — a `tracing::error!` in
    /// every build (the soak harness and production run release, where
    /// `debug_assert!` is a no-op) plus a hard `debug_assert!` in debug/test —
    /// rather than a silent double-count of the Prometheus counter the harness
    /// scrapes. Written with `Release` / read with `Acquire` so the boot seed
    /// reliably observes a prior arm even if the two ever run on different
    /// threads.
    add_fired: AtomicBool,
}

impl ProcessGauge {
    /// Creates a zeroed, un-armed gauge.
    ///
    /// `const` so the process instance is a plain static initialiser with no
    /// `OnceLock` or lazy machinery on the read path.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            bytes: AtomicU64::new(0),
            add_fired: AtomicBool::new(false),
        }
    }

    /// Adds `n` bytes to the gauge and emits the same delta to both exported
    /// Prometheus series: the `topgun_ormap_tombstone_bytes_total` monotonic
    /// creation-rate counter (mirrors the `topgun_operations_total` emit pattern
    /// in `service/middleware/metrics.rs`), and the
    /// `topgun_ormap_tombstone_bytes` gauge — the decrementable series a prune
    /// path can move back down, and the one the soak monitor's plateau/slope fit
    /// reads over `GET /metrics`.
    pub fn add(&self, n: u64) {
        // Arm the tripwire that makes the boot-seed dual-write asymmetry fail
        // loud (see `add_fired`). A single store on the write path is negligible
        // next to the map mutation that precedes it; `Release` pairs with the
        // boot seed's `Acquire` load so the seed reliably observes this arm
        // cross-thread.
        self.add_fired.store(true, Ordering::Release);
        self.bytes.fetch_add(n, Ordering::Relaxed);
        metrics::counter!("topgun_ormap_tombstone_bytes_total").increment(n);
        // Precision loss only above 2^53 bytes of tombstone data on one process
        // — not a real-world concern for this monitoring signal.
        #[allow(clippy::cast_precision_loss)]
        metrics::gauge!("topgun_ormap_tombstone_bytes").increment(n as f64);
    }

    /// Subtracts `n` bytes from the gauge and mirrors the same decrement onto
    /// the exported `topgun_ormap_tombstone_bytes` Prometheus gauge — the
    /// byte-for-byte counterpart of [`ProcessGauge::add`]'s increment.
    ///
    /// Unlike the `topgun_ormap_tombstone_bytes_total` counter (which follows
    /// the `_total` *monotonic* convention and therefore cannot legally
    /// decrease), `topgun_ormap_tombstone_bytes` has no such constraint: it is a
    /// plain Prometheus gauge, so this decrement is externally visible over
    /// `GET /metrics` without needing the in-process accessor — the surface an
    /// out-of-process soak monitor actually scrapes.
    ///
    /// `fetch_sub` wraps on underflow rather than panicking — acceptable for a
    /// monitoring counter, and it keeps an underflow regression observable
    /// rather than masking it behind saturation.
    pub fn sub(&self, n: u64) {
        self.bytes.fetch_sub(n, Ordering::Relaxed);
        // Precision loss only above 2^53 bytes of tombstone data on one process
        // — not a real-world concern for this monitoring signal.
        #[allow(clippy::cast_precision_loss)]
        metrics::gauge!("topgun_ormap_tombstone_bytes").decrement(n as f64);
    }

    /// Reads the gauge's current value.
    #[must_use]
    pub fn read(&self) -> u64 {
        self.bytes.load(Ordering::Relaxed)
    }

    /// Re-baselines the gauge to an absolute `total`.
    ///
    /// This is the **only** absolute-set path for the gauge. It exists
    /// exclusively for the one-time startup reconciliation that runs after WAL
    /// recovery completes (see `bin/topgun_server.rs`): the process-local atomic
    /// and both exported Prometheus series
    /// (`topgun_ormap_tombstone_bytes_total` and `topgun_ormap_tombstone_bytes`)
    /// reset to zero/absent on every process start and never re-count rehydrated
    /// (redb-persisted) tombstones, so without this boot seed the scraped series
    /// sawtooths back to 0 on every `kill -9` restart and the cross-restart leak
    /// becomes invisible. Seeding from the true reconciled corpus (rather than a
    /// literal `0`) means a genuine leak still shows as net upward drift from a
    /// real starting point, and a monitor that only trusts non-zero samples sees
    /// one from the first scrape after boot. It MUST NEVER be called on the hot
    /// read/write path — only once, at boot, before the listener accepts
    /// connections.
    ///
    /// It performs THREE writes, in order, because they are **separate sinks**:
    ///  1. `bytes.store(total)` re-baselines the in-process `AtomicU64` — an
    ///     absolute store, never `fetch_add`: a per-rehydration increment would
    ///     reintroduce the eviction double-count the gauge's cardinal rule
    ///     forbids.
    ///  2. `metrics::counter!(...).increment(total)` seeds the monotonic
    ///     `_total` Prometheus counter, a *different* sink living in the process
    ///     `PrometheusHandle` recorder; on a fresh process it starts at
    ///     0/absent, so a single `increment(total)` from zero lands the exported
    ///     series at `total` while staying a legal monotonic-from-zero counter (a
    ///     Prometheus counter has no `.set`/`.store`).
    ///  3. `metrics::gauge!(...).set(total)` re-baselines the decrementable
    ///     `topgun_ormap_tombstone_bytes` gauge — the series the soak monitor's
    ///     plateau/slope fit actually scrapes over `GET /metrics`. Unlike the
    ///     counter this is a plain absolute `.set`, so it carries no additive
    ///     double-count risk and can be called safely even if this boot seed ever
    ///     ran more than once.
    pub fn set(&self, total: u64) {
        self.bytes.store(total, Ordering::Relaxed);
        // The Prometheus increment below is additive; correct only on a
        // fresh-zero recorder. No add call site is reachable before this boot
        // seed, so the tripwire must still be un-armed here — otherwise the
        // counter the harness scrapes would silently double-count. Fail loud in
        // EVERY build: the soak harness and production run release, where
        // `debug_assert!` alone is a no-op, so an error log carries the signal
        // there while the debug_assert hard-fails tests.
        let armed = self.add_fired.load(Ordering::Acquire);
        if armed {
            tracing::error!(
                target: "topgun_server::bootstrap",
                "set_tombstone_bytes boot seed ran after add_tombstone_bytes — the additive \
                 Prometheus counter will double-count; the recovery→ready gauge-window invariant \
                 was violated by a reachable pre-set_ready write path"
            );
        }
        debug_assert!(
            !armed,
            "set_tombstone_bytes boot seed ran after add_tombstone_bytes — the additive \
             Prometheus counter would double-count"
        );
        metrics::counter!("topgun_ormap_tombstone_bytes_total").increment(total);
        // Absolute re-baseline of the decrementable gauge — no additive-race
        // hazard here since `.set` (unlike the counter's `.increment`)
        // overwrites rather than accumulates.
        #[allow(clippy::cast_precision_loss)]
        metrics::gauge!("topgun_ormap_tombstone_bytes").set(total as f64);
    }

    /// Reports whether an add has fired on **this instance**.
    ///
    /// Instance-scoped by construction: it can only ever report on its receiver,
    /// so a test binding a private gauge observes a tripwire no other test can
    /// influence.
    #[cfg(test)]
    pub(crate) fn armed(&self) -> bool {
        self.add_fired.load(Ordering::Acquire)
    }
}

impl Default for ProcessGauge {
    fn default() -> Self {
        Self::new()
    }
}

/// Every method here MUST stay a one-line delegation to the inherent method,
/// with no added logic.
///
/// The resolver is `cfg`-split: in release it hands the closure a
/// `&ProcessGauge`, where an inherent method always wins method resolution; in
/// test builds it hands over a `&dyn TombstoneGaugeSink`, where only these trait
/// methods are visible. The two builds therefore call *different* methods, and
/// stay behaviourally identical only for as long as this impl is a pure
/// delegation. Anything added here — an extra emit, a counter, a debug assert —
/// would apply in test builds and not in release, and no test in this crate
/// could observe the divergence, because tests only ever exercise the test arm.
impl TombstoneGaugeSink for ProcessGauge {
    fn add(&self, n: u64) {
        Self::add(self, n);
    }

    fn sub(&self, n: u64) {
        Self::sub(self, n);
    }

    fn read(&self) -> u64 {
        Self::read(self)
    }

    fn set(&self, total: u64) {
        Self::set(self, total);
    }
}

/// The one gauge production ever writes to.
pub(crate) static PROCESS_GAUGE: ProcessGauge = ProcessGauge::new();

/// A private, test-only sink: a bare counter with no Prometheus emission.
///
/// It carries **no tripwire field at all**, which makes "an isolated add cannot
/// arm a tripwire" a property the compiler checks rather than an assertion some
/// test has to make about global state.
#[cfg(test)]
pub(crate) struct IsolatedGauge {
    bytes: AtomicU64,
}

#[cfg(test)]
impl IsolatedGauge {
    /// Creates a zeroed isolated sink.
    pub(crate) const fn new() -> Self {
        Self {
            bytes: AtomicU64::new(0),
        }
    }
}

#[cfg(test)]
impl TombstoneGaugeSink for IsolatedGauge {
    fn add(&self, n: u64) {
        self.bytes.fetch_add(n, Ordering::Relaxed);
    }

    /// `fetch_sub` rather than a saturating subtraction, matching production, so
    /// an underflow regression stays observable instead of being masked.
    fn sub(&self, n: u64) {
        self.bytes.fetch_sub(n, Ordering::Relaxed);
    }

    fn read(&self) -> u64 {
        self.bytes.load(Ordering::Relaxed)
    }

    fn set(&self, total: u64) {
        self.bytes.store(total, Ordering::Relaxed);
    }
}

#[cfg(test)]
tokio::task_local! {
    /// Ambient sink override for the current task and everything it awaits
    /// inline.
    ///
    /// A `tokio::task_local` rather than a `std::thread_local`: `#[tokio::test]`
    /// and the proptest bridge run on multi-thread runtimes that may move a task
    /// to a different worker at every `.await`, so a thread-local override would
    /// be silently dropped mid-test — and silence is precisely the failure mode
    /// this seam exists to remove.
    static GAUGE_SINK: Arc<dyn TombstoneGaugeSink>;
}

/// Resolves the gauge sink for the current context and hands it to `f`.
///
/// Release builds resolve statically to [`PROCESS_GAUGE`], so the call
/// monomorphises and inlines down to the same atomic operation it has always
/// performed — no branch, no vtable.
#[cfg(not(test))]
pub(crate) fn with_sink<R>(f: impl FnOnce(&ProcessGauge) -> R) -> R {
    f(&PROCESS_GAUGE)
}

/// Resolves the gauge sink for the current context and hands it to `f`.
///
/// Prefers a task-local override when one is bound, else [`PROCESS_GAUGE`].
///
/// A missing override is **not** an error and must never panic: the gauge is
/// reachable from synchronous, runtime-less contexts (`evict_lru`, the boot
/// seed), and the overwhelming majority of this crate's tests perform OR-removes
/// or inbound syncs with no gauge scope at all. Routing that unscoped traffic to
/// the process gauge is the designed behaviour — it is exactly what makes a
/// scoped assertion immune to it.
#[cfg(test)]
pub(crate) fn with_sink<R>(f: impl FnOnce(&dyn TombstoneGaugeSink) -> R) -> R {
    match GAUGE_SINK.try_with(Arc::clone) {
        Ok(sink) => f(&*sink),
        Err(_) => f(&PROCESS_GAUGE),
    }
}

/// Runs `f` with `sink` bound as the ambient gauge sink.
///
/// The override propagates to everything awaited **inline on the same task**. It
/// does **NOT** propagate across `tokio::spawn`: a write driven through a spawned
/// task observes the process gauge instead, and a scoped read of it would come
/// back as 0. Any future test that drives a gauge write through a spawned task
/// must account for that rather than assume the scope reaches it.
#[cfg(test)]
pub(crate) async fn with_gauge_sink<F, R>(sink: Arc<dyn TombstoneGaugeSink>, f: F) -> R
where
    F: Future<Output = R>,
{
    GAUGE_SINK.scope(sink, f).await
}

/// Runs `f` against a fresh [`IsolatedGauge`], returning its output alongside
/// the sink's final value.
///
/// The sink starts at zero, so the returned value **is** the net delta of every
/// gauge write made inside the scope — and only those. The same inline-only
/// propagation contract as [`with_gauge_sink`] applies: writes made on a
/// `tokio::spawn`ed task land on the process gauge and are not counted here.
#[cfg(test)]
pub(crate) async fn with_isolated_gauge<F, R>(f: F) -> (R, u64)
where
    F: Future<Output = R>,
{
    let sink = Arc::new(IsolatedGauge::new());
    let observer = Arc::clone(&sink);
    let out = with_gauge_sink(sink, f).await;
    (out, observer.read())
}

#[cfg(test)]
mod tests {
    use super::{
        with_gauge_sink, with_isolated_gauge, with_sink, IsolatedGauge, ProcessGauge,
        TombstoneGaugeSink,
    };
    use crate::storage::record::{add_tombstone_bytes, tombstone_bytes};
    use std::sync::Arc;

    #[tokio::test]
    async fn isolated_scope_reports_its_own_net_delta() {
        let (out, delta) = with_isolated_gauge(async {
            with_sink(|s| s.add(7));
            with_sink(|s| s.sub(2));
            with_sink(|s| s.read())
        })
        .await;

        assert_eq!(out, 5, "a scoped read must observe the scoped sink");
        assert_eq!(delta, 5, "the scope's final value is its net delta");
    }

    /// Enforces the pure-delegation invariant the trait impl documents, rather
    /// than leaving it to a comment nobody re-checks.
    ///
    /// Release resolves `s.add(n)` to the inherent method and test builds
    /// resolve it to the trait method, so the two builds agree only while the
    /// trait impl delegates verbatim. Tests exercise the trait arm exclusively,
    /// which means a divergence introduced here would be invisible to every
    /// other test in the crate — including the ones this module exists to make
    /// trustworthy. Driving both arms over identical instances and comparing
    /// the observable state turns that silent divergence into a red test.
    #[test]
    fn the_trait_impl_delegates_verbatim_to_the_inherent_methods() {
        let inherent = ProcessGauge::new();
        let via_trait = ProcessGauge::new();
        let dynamic: &dyn TombstoneGaugeSink = &via_trait;

        // Same op sequence, one arm per dispatch form.
        inherent.add(9);
        dynamic.add(9);
        ProcessGauge::sub(&inherent, 4);
        dynamic.sub(4);

        assert_eq!(
            ProcessGauge::read(&inherent),
            dynamic.read(),
            "inherent and trait dispatch must leave identical byte counts"
        );
        assert_eq!(
            inherent.armed(),
            via_trait.armed(),
            "inherent and trait dispatch must arm the tripwire identically"
        );
    }

    /// Pins the binding helper itself: a caller-supplied sink receives the
    /// scope's writes, and its tripwire is observable on that instance alone —
    /// never through a process-global read that every unmarked OR-remove in the
    /// crate would arm.
    #[tokio::test]
    async fn an_explicitly_bound_sink_receives_the_scope_s_writes() {
        let sink = Arc::new(ProcessGauge::new());
        let observer = Arc::clone(&sink);
        assert!(!observer.armed(), "a fresh gauge starts un-armed");

        with_gauge_sink(sink, async {
            with_sink(|s| s.add(3));
        })
        .await;

        assert_eq!(observer.read(), 3);
        assert!(observer.armed(), "an add arms the receiving instance");
    }

    /// Adversarial proof that a scoped assertion is immune to concurrent
    /// foreign traffic, with an in-band negative control proving the foreign
    /// traffic was real.
    ///
    /// The foreign writes go through `tokio::spawn`, which the task-local
    /// override deliberately does not cross, so they land on the process gauge —
    /// exactly the contention a gauge assertion has to survive. They are joined
    /// *inside* the scope so they are guaranteed to have landed before it
    /// closes; an un-joined spawn would let the test pass while proving nothing.
    ///
    /// The process-gauge readings are taken **outside** the scope on purpose:
    /// inside it, `tombstone_bytes()` resolves to the isolated sink, so a
    /// reading there would measure the wrong sink and collapse (b) into a
    /// restatement of (a). And (b) is a lower bound, never an equality — the
    /// process gauge is shared with the whole crate, so pinning it exactly would
    /// re-create the order-dependence this seam exists to remove.
    #[tokio::test]
    async fn a_scope_ignores_concurrent_foreign_traffic_on_the_process_gauge() {
        const FOREIGN_TASKS: u64 = 4;
        const FOREIGN_PER_TASK: u64 = 25;
        const FOREIGN_TOTAL: u64 = FOREIGN_TASKS * FOREIGN_PER_TASK;
        const OWN: u64 = 11;

        let before = tombstone_bytes();

        let ((), delta) = with_isolated_gauge(async {
            let foreign: Vec<_> = (0..FOREIGN_TASKS)
                .map(|_| tokio::spawn(async { add_tombstone_bytes(FOREIGN_PER_TASK) }))
                .collect();

            add_tombstone_bytes(OWN);

            for task in foreign {
                task.await.expect("foreign gauge task must not panic");
            }
        })
        .await;

        let after = tombstone_bytes();

        assert_eq!(
            delta, OWN,
            "the scoped delta must count only the scope's own writes"
        );
        // Phrased as an addition rather than `after - before` so a concurrent
        // prune elsewhere in the suite cannot underflow the subtraction into a
        // panic before the assertion gets to report anything.
        assert!(
            after >= before.saturating_add(FOREIGN_TOTAL),
            "the foreign traffic must have landed on the process gauge \
             (before {before}, after {after}, expected a rise of at least {FOREIGN_TOTAL})"
        );
    }

    /// The boot-seed tripwire still arms when an add reaches the gauge through
    /// `record.rs`'s public API.
    ///
    /// Both reads are taken on a private `ProcessGauge` bound for this scope
    /// alone, never on the process singleton: every unmarked OR-remove in the
    /// crate arms that one, so a global read would be order-dependent by
    /// construction. `set()` is never called here — on an armed instance it
    /// trips the boot-seed `debug_assert!`.
    #[tokio::test]
    async fn an_add_through_the_public_api_arms_its_own_gauge_s_tripwire() {
        let sink = Arc::new(ProcessGauge::new());
        let observer = Arc::clone(&sink);

        assert!(
            !observer.armed(),
            "a freshly-constructed gauge must start un-armed"
        );

        with_gauge_sink(sink, async {
            add_tombstone_bytes(6);
        })
        .await;

        assert_eq!(observer.read(), 6, "the bound instance received the add");
        assert!(
            observer.armed(),
            "an add routed through the public API must arm the receiving instance"
        );
    }

    #[test]
    fn an_isolated_sub_wraps_rather_than_saturating() {
        // Wrapping keeps an underflow regression observable instead of hiding it
        // behind a floor of zero.
        let sink = IsolatedGauge::new();
        sink.sub(1);
        assert_eq!(sink.read(), u64::MAX);
    }
}
