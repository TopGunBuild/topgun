//! The `KvEngine` conformance suite: one set of cases every engine runs
//! unchanged (`TG-ENG-001`, `TG-ENG-002`, `TG-ENG-003`).
//!
//! An engine joins by one factory and one invocation line per macro arm. No
//! shared case names an engine, looks at an engine's internals, sleeps or
//! asserts a timing. The engine-specific cases at the end check what only one
//! engine can show (the reference engine's crash hooks, redb's commit and
//! error paths).

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};

use proptest::prelude::*;

use super::kv_engine::{
    BatchOp, KeyRange, KvCapability, KvEngine, KvError, KvSnapshot, MemKvEngine, PrefixPage,
    WriteBatch, WriteOptions,
};

/// Builds an engine for one test.
trait KvTestFactory {
    type Engine: KvEngine;
    /// Keeps the engine's files alive for the test (a `tempfile::TempDir`); `()` for in-memory engines.
    type Guard;
    fn make() -> (Self::Engine, Self::Guard);
}

/// A factory whose engines live on files that can be opened again.
trait KvReopenFactory: KvTestFactory {
    /// A new engine on the files `guard` keeps; called after the previous engine was shut down and dropped.
    fn reopen(guard: &Self::Guard) -> Self::Engine;
}

struct MemFactory;

impl KvTestFactory for MemFactory {
    type Engine = MemKvEngine;
    type Guard = ();
    fn make() -> (Self::Engine, Self::Guard) {
        (MemKvEngine::new(), ())
    }
}

#[cfg(feature = "redb")]
struct RedbMemFactory;

#[cfg(feature = "redb")]
impl KvTestFactory for RedbMemFactory {
    type Engine = super::RedbKvEngine;
    type Guard = ();
    fn make() -> (Self::Engine, Self::Guard) {
        let engine =
            super::RedbKvEngine::with_backend(::redb::backends::InMemoryBackend::new()).unwrap();
        (engine, ())
    }
}

#[cfg(feature = "redb")]
struct RedbFileFactory;

#[cfg(feature = "redb")]
impl KvTestFactory for RedbFileFactory {
    type Engine = super::RedbKvEngine;
    type Guard = tempfile::TempDir;
    fn make() -> (Self::Engine, Self::Guard) {
        let dir = tempfile::tempdir().unwrap();
        let engine = super::RedbKvEngine::open(&dir.path().join("kv.redb")).unwrap();
        (engine, dir)
    }
}

#[cfg(feature = "redb")]
impl KvReopenFactory for RedbFileFactory {
    fn reopen(guard: &Self::Guard) -> Self::Engine {
        super::RedbKvEngine::open(&guard.path().join("kv.redb")).unwrap()
    }
}

// ---- helpers ----------------------------------------------------------------

type Model = BTreeMap<Vec<u8>, Vec<u8>>;
type Entries = Vec<(Vec<u8>, Vec<u8>)>;

const SYNC: WriteOptions = WriteOptions { sync: true };
const NOSYNC: WriteOptions = WriteOptions { sync: false };

fn put<E: KvEngine>(engine: &E, key: &[u8], value: &[u8]) {
    let mut batch = WriteBatch::new();
    batch.put(key, value);
    engine.write(batch, NOSYNC).unwrap();
}

fn snapshot_entries(snapshot: &dyn KvSnapshot, range: KeyRange) -> Entries {
    snapshot.iter(range).unwrap().map(Result::unwrap).collect()
}

/// The engine's full content, through a fresh snapshot.
fn all<E: KvEngine>(engine: &E) -> Entries {
    let snapshot = engine.snapshot().unwrap();
    snapshot_entries(snapshot.as_ref(), KeyRange::default())
}

fn entries(model: &Model) -> Entries {
    model.iter().map(|(k, v)| (k.clone(), v.clone())).collect()
}

fn apply(model: &mut Model, ops: &[BatchOp]) {
    for op in ops {
        match op {
            BatchOp::Put { key, value } => {
                model.insert(key.clone(), value.clone());
            }
            BatchOp::Delete { key } => {
                model.remove(key);
            }
            BatchOp::DeleteRange { start, end } => {
                model.retain(|k, _| !(k >= start && k < end));
            }
        }
    }
}

fn batch_is_valid(ops: &[BatchOp]) -> bool {
    ops.iter().all(|op| match op {
        BatchOp::DeleteRange { start, end } => start <= end,
        _ => true,
    })
}

fn batch_of(ops: &[BatchOp]) -> WriteBatch {
    let mut batch = WriteBatch::new();
    for op in ops {
        match op {
            BatchOp::Put { key, value } => batch.put(key.clone(), value.clone()),
            BatchOp::Delete { key } => batch.delete(key.clone()),
            BatchOp::DeleteRange { start, end } => batch.delete_range(start.clone(), end.clone()),
        }
    }
    batch
}

fn bounded(start: &[u8], end: &[u8]) -> KeyRange {
    KeyRange {
        start: Some(start.to_vec()),
        end: Some(end.to_vec()),
    }
}

fn pair(key: &[u8], value: &[u8]) -> (Vec<u8>, Vec<u8>) {
    (key.to_vec(), value.to_vec())
}

fn keys(list: &[&[u8]]) -> Vec<Vec<u8>> {
    list.iter().map(|k| k.to_vec()).collect()
}

fn page_keys(page: &PrefixPage) -> Vec<Vec<u8>> {
    page.entries.iter().map(|(k, _)| k.clone()).collect()
}

/// The page the contract prescribes for `model` (the page rule of `scan_prefix`).
fn model_page(
    model: &Model,
    prefix: &[u8],
    resume_after: Option<&[u8]>,
    limit: usize,
) -> PrefixPage {
    let entries: Entries = model
        .iter()
        .filter(|(k, _)| k.starts_with(prefix) && resume_after.is_none_or(|r| k.as_slice() > r))
        .take(limit)
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    let resume_after = if entries.len() == limit {
        entries.last().map(|(k, _)| k.clone())
    } else {
        None
    };
    PrefixPage {
        entries,
        resume_after,
    }
}

/// A deterministic xorshift generator, so the "random" keys are the same in every run.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    fn bytes(&mut self, max_len: u64) -> Vec<u8> {
        let len = self.next() % (max_len + 1);
        (0..len).map(|_| self.next().to_le_bytes()[0]).collect()
    }
}

// ---- shared cases -----------------------------------------------------------

fn case_c01<F: KvTestFactory>() {
    let (engine, _guard) = F::make();
    assert_eq!(engine.get(b"absent").unwrap(), None);
    assert_eq!(engine.get(b"").unwrap(), None);
    put(&engine, b"other", b"v");
    assert_eq!(engine.get(b"absent").unwrap(), None);
}

fn case_c02<F: KvTestFactory>() {
    let (engine, _guard) = F::make();
    put(&engine, b"a", b"1");
    assert_eq!(engine.get(b"a").unwrap(), Some(b"1".to_vec()));
    put(&engine, b"a", b"2");
    assert_eq!(engine.get(b"a").unwrap(), Some(b"2".to_vec()));
    put(&engine, b"", b"empty key");
    assert_eq!(engine.get(b"").unwrap(), Some(b"empty key".to_vec()));
    assert_eq!(
        all(&engine),
        vec![pair(b"", b"empty key"), pair(b"a", b"2")]
    );
}

fn case_c03<F: KvTestFactory>() {
    let (engine, _guard) = F::make();
    put(&engine, b"a", b"1");
    put(&engine, b"b", b"1");
    engine.delete(b"a", NOSYNC).unwrap();
    assert_eq!(engine.get(b"a").unwrap(), None);
    engine.delete(b"never", SYNC).unwrap();
    assert_eq!(all(&engine), vec![pair(b"b", b"1")]);
}

fn case_c04<F: KvTestFactory>() {
    let (engine, _guard) = F::make();
    let mut model = Model::new();
    let batches: Vec<Vec<BatchOp>> = vec![
        // put then put of one key: the later wins
        vec![
            BatchOp::Put {
                key: b"k1".to_vec(),
                value: b"1".to_vec(),
            },
            BatchOp::Put {
                key: b"k1".to_vec(),
                value: b"2".to_vec(),
            },
        ],
        // put then delete
        vec![
            BatchOp::Put {
                key: b"k2".to_vec(),
                value: b"1".to_vec(),
            },
            BatchOp::Delete {
                key: b"k2".to_vec(),
            },
        ],
        // delete then put
        vec![
            BatchOp::Delete {
                key: b"k3".to_vec(),
            },
            BatchOp::Put {
                key: b"k3".to_vec(),
                value: b"1".to_vec(),
            },
        ],
        // puts then a range delete covering them, then a put inside the range
        vec![
            BatchOp::Put {
                key: b"r1".to_vec(),
                value: b"1".to_vec(),
            },
            BatchOp::Put {
                key: b"r2".to_vec(),
                value: b"1".to_vec(),
            },
            BatchOp::DeleteRange {
                start: b"r0".to_vec(),
                end: b"r9".to_vec(),
            },
            BatchOp::Put {
                key: b"r5".to_vec(),
                value: b"1".to_vec(),
            },
        ],
    ];
    for ops in &batches {
        engine.write(batch_of(ops), NOSYNC).unwrap();
        apply(&mut model, ops);
        assert_eq!(all(&engine), entries(&model));
    }
    assert_eq!(engine.get(b"k1").unwrap(), Some(b"2".to_vec()));
    assert_eq!(engine.get(b"k2").unwrap(), None);
    assert_eq!(engine.get(b"k3").unwrap(), Some(b"1".to_vec()));
    assert_eq!(engine.get(b"r1").unwrap(), None);
    assert_eq!(engine.get(b"r5").unwrap(), Some(b"1".to_vec()));
}

fn case_c05<F: KvTestFactory>() {
    let (engine, _guard) = F::make();
    let mut batch = WriteBatch::new();
    batch.put(b"a", b"1");
    batch.put(b"b", b"1");
    batch.delete_range(b"z", b"a");
    let result = engine.write(batch, SYNC);
    assert!(
        matches!(result, Err(KvError::InvalidArgument(_))),
        "{result:?}"
    );
    assert_eq!(engine.get(b"a").unwrap(), None);
    assert_eq!(engine.get(b"b").unwrap(), None);
    assert_eq!(all(&engine), Entries::new());
    let result = engine.delete_range(b"z", b"a", NOSYNC);
    assert!(
        matches!(result, Err(KvError::InvalidArgument(_))),
        "{result:?}"
    );
}

fn case_c06<F: KvTestFactory>() {
    let (engine, _guard) = F::make();
    put(&engine, b"x", b"1");
    let before = all(&engine);
    engine.write(WriteBatch::new(), NOSYNC).unwrap();
    engine.write(WriteBatch::new(), SYNC).unwrap();
    assert_eq!(all(&engine), before);
}

fn case_c07<F: KvTestFactory>() {
    let (engine, _guard) = F::make();
    let mut model = Model::new();
    for key in [&b""[..], &[0x00], b"a", b"b", b"c", &[0xFF], &[0xFF, 0xFF]] {
        put(&engine, key, b"v");
        model.insert(key.to_vec(), b"v".to_vec());
    }
    let ranges: [(&[u8], &[u8]); 4] = [
        (b"a", b"c"),
        (&[0x00], &[0xFF]),
        (&[0xFF], &[0xFF, 0xFF]),
        (b"", &[0x00]),
    ];
    for (i, (start, end)) in ranges.iter().enumerate() {
        // Alternate the trait method and a batch op, so both paths are held to the bounds.
        if i % 2 == 0 {
            engine.delete_range(start, end, NOSYNC).unwrap();
        } else {
            let mut batch = WriteBatch::new();
            batch.delete_range(*start, *end);
            engine.write(batch, NOSYNC).unwrap();
        }
        apply(
            &mut model,
            &[BatchOp::DeleteRange {
                start: start.to_vec(),
                end: end.to_vec(),
            }],
        );
        assert_eq!(
            all(&engine),
            entries(&model),
            "after deleting [{start:?}, {end:?})"
        );
    }
    assert_eq!(all(&engine), vec![pair(&[0xFF, 0xFF], b"v")]);
}

fn case_c08<F: KvTestFactory>() {
    let (engine, _guard) = F::make();
    put(&engine, b"", b"v");
    put(&engine, b"a", b"v");
    put(&engine, b"b", b"v");
    let before = all(&engine);
    engine.delete_range(b"a", b"a", NOSYNC).unwrap();
    engine.delete_range(b"", b"", SYNC).unwrap();
    let mut batch = WriteBatch::new();
    batch.delete_range(b"b", b"b");
    engine.write(batch, NOSYNC).unwrap();
    assert_eq!(all(&engine), before);
}

fn case_c09<F: KvTestFactory>() {
    let (engine, _guard) = F::make();
    let mut model = Model::new();
    let mut batch = WriteBatch::new();
    for key in [&b""[..], &[0x00], b"a", b"a\0", b"ab", &[0xFF]] {
        batch.put(key, key);
        model.insert(key.to_vec(), key.to_vec());
    }
    engine.write(batch, NOSYNC).unwrap();
    let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
    for _ in 0..10 {
        let mut batch = WriteBatch::new();
        for _ in 0..100 {
            let key = rng.bytes(8);
            let value = rng.bytes(4);
            batch.put(key.clone(), value.clone());
            model.insert(key, value);
        }
        engine.write(batch, NOSYNC).unwrap();
    }
    let got = all(&engine);
    assert!(
        got.windows(2).all(|w| w[0].0 < w[1].0),
        "not strictly ascending"
    );
    assert_eq!(got, entries(&model));
    let head: Vec<Vec<u8>> = got.iter().take(1).map(|(k, _)| k.clone()).collect();
    assert_eq!(head, vec![Vec::<u8>::new()]);
}

fn case_c10<F: KvTestFactory>() {
    let (engine, _guard) = F::make();
    for key in [b"a", b"b", b"c", b"d"] {
        put(&engine, key, b"v");
    }
    let snapshot = engine.snapshot().unwrap();
    let got = |start: Option<&[u8]>, end: Option<&[u8]>| -> Vec<Vec<u8>> {
        let range = KeyRange {
            start: start.map(<[u8]>::to_vec),
            end: end.map(<[u8]>::to_vec),
        };
        snapshot_entries(snapshot.as_ref(), range)
            .into_iter()
            .map(|(k, _)| k)
            .collect()
    };
    assert_eq!(got(None, None), keys(&[b"a", b"b", b"c", b"d"]));
    assert_eq!(got(Some(b"b"), None), keys(&[b"b", b"c", b"d"]));
    assert_eq!(got(None, Some(b"c")), keys(&[b"a", b"b"]));
    assert_eq!(got(Some(b"b"), Some(b"d")), keys(&[b"b", b"c"]));
    assert_eq!(got(Some(b"b"), Some(b"b")), Vec::<Vec<u8>>::new());
    assert_eq!(got(Some(b"bb"), Some(b"d")), keys(&[b"c"]));
    assert_eq!(got(Some(b"0"), Some(b"z")), keys(&[b"a", b"b", b"c", b"d"]));
    assert!(matches!(
        snapshot.iter(bounded(b"d", b"b")),
        Err(KvError::InvalidArgument(_))
    ));
}

fn case_c11<F: KvTestFactory>() {
    let (engine, _guard) = F::make();
    for key in [&b"a"[..], b"b", b"c", b"r1", b"r2"] {
        put(&engine, key, b"1");
    }
    let before = all(&engine);
    let snapshot = engine.snapshot().unwrap();
    put(&engine, b"a", b"2");
    engine.delete(b"b", NOSYNC).unwrap();
    engine.delete_range(b"r0", b"r9", SYNC).unwrap();
    put(&engine, b"d", b"1");
    assert_eq!(snapshot.get(b"a").unwrap(), Some(b"1".to_vec()));
    assert_eq!(snapshot.get(b"b").unwrap(), Some(b"1".to_vec()));
    assert_eq!(snapshot.get(b"r1").unwrap(), Some(b"1".to_vec()));
    assert_eq!(snapshot.get(b"d").unwrap(), None);
    assert_eq!(
        snapshot_entries(snapshot.as_ref(), KeyRange::default()),
        before
    );
    assert_eq!(
        all(&engine),
        vec![pair(b"a", b"2"), pair(b"c", b"1"), pair(b"d", b"1")]
    );
}

fn case_c12<F: KvTestFactory>() {
    let (engine, _guard) = F::make();
    for i in 0..10u8 {
        put(&engine, &[b'k', b'0' + i], b"v");
    }
    let before = all(&engine);
    let snapshot = engine.snapshot().unwrap();
    let mut first = snapshot.iter(KeyRange::default()).unwrap();
    let mut second = snapshot.iter(KeyRange::default()).unwrap();
    let mut seen_first: Entries = (&mut first).take(3).map(Result::unwrap).collect();
    put(&engine, b"k05x", b"new");
    engine.delete(b"k7", NOSYNC).unwrap();
    let seen_second: Entries = (&mut second).take(5).map(Result::unwrap).collect();
    engine.delete_range(b"k0", b"k9z", NOSYNC).unwrap();
    put(&engine, b"k95", b"new");
    seen_first.extend(first.map(Result::unwrap));
    let mut seen_second = seen_second;
    seen_second.extend(second.map(Result::unwrap));
    assert_eq!(seen_first, before);
    assert_eq!(seen_second, before);
    assert_eq!(all(&engine), vec![pair(b"k95", b"new")]);
}

fn case_c13<F: KvTestFactory>() {
    let (engine, _guard) = F::make();
    let outside: [&[u8]; 6] = [b"", b"o", b"oz\xff", b"q", b"q\x00", b"r"];
    let inside: [&[u8]; 5] = [b"p", b"p\x00", b"pa", b"p\xff", b"p\xff\xff"];
    for key in outside.iter().chain(inside.iter()) {
        put(&engine, key, key);
    }
    let page = engine.scan_prefix(b"p", None, 100).unwrap();
    let expected: Entries = inside.iter().map(|k| pair(k, k)).collect();
    assert_eq!(page.entries, expected);
    assert_eq!(page.resume_after, None);
}

fn case_c14<F: KvTestFactory>() {
    let (engine, _guard) = F::make();
    let mut expected = Vec::new();
    for chunk in 0..10 {
        let mut batch = WriteBatch::new();
        for i in chunk * 100..(chunk + 1) * 100 {
            let key = format!("k/{i:04}").into_bytes();
            batch.put(key.clone(), b"v");
            expected.push(key);
        }
        engine.write(batch, NOSYNC).unwrap();
    }
    for key in [&b"j"[..], b"k", b"k.", b"k0", b"l"] {
        put(&engine, key, b"outside");
    }
    for limit in [1usize, 2, 7, 1_000, 1_001] {
        let expected_calls = 1_000 / limit + 1;
        let mut walked = Vec::new();
        let mut resume: Option<Vec<u8>> = None;
        let mut calls = 0;
        loop {
            calls += 1;
            assert!(
                calls <= expected_calls,
                "limit {limit}: more than {expected_calls} calls"
            );
            let page = engine.scan_prefix(b"k/", resume.as_deref(), limit).unwrap();
            let full = page.entries.len() == limit;
            if full {
                assert_eq!(
                    page.resume_after,
                    page.entries.last().map(|(k, _)| k.clone())
                );
            } else {
                assert_eq!(
                    page.entries.len(),
                    1_000 % limit,
                    "limit {limit}: last page size"
                );
                assert_eq!(page.resume_after, None);
            }
            walked.extend(page_keys(&page));
            match page.resume_after {
                Some(r) => resume = Some(r),
                None => break,
            }
        }
        assert_eq!(calls, expected_calls, "limit {limit}: calls");
        assert_eq!(walked, expected, "limit {limit}: keys");
    }
}

fn case_c15<F: KvTestFactory>() {
    let (engine, _guard) = F::make();
    let all_keys: [&[u8]; 11] = [
        b"",
        &[0x61, 0xFE],
        &[0x61, 0xFF],
        &[0x61, 0xFF, 0x00],
        &[0x61, 0xFF, 0xFF],
        &[0x61, 0xFF, 0xFF, 0x01],
        &[0x62],
        &[0xFF],
        &[0xFF, 0xFE],
        &[0xFF, 0xFF],
        &[0xFF, 0xFF, 0xFF],
    ];
    for key in all_keys {
        put(&engine, key, b"v");
    }
    let scan = |prefix: &[u8]| page_keys(&engine.scan_prefix(prefix, None, 100).unwrap());
    assert_eq!(
        scan(&[0x61, 0xFF]),
        keys(&[
            &[0x61, 0xFF],
            &[0x61, 0xFF, 0x00],
            &[0x61, 0xFF, 0xFF],
            &[0x61, 0xFF, 0xFF, 0x01]
        ])
    );
    assert_eq!(
        scan(&[0xFF, 0xFF]),
        keys(&[&[0xFF, 0xFF], &[0xFF, 0xFF, 0xFF]])
    );
    assert_eq!(
        scan(&[0xFF]),
        keys(&[&[0xFF], &[0xFF, 0xFE], &[0xFF, 0xFF], &[0xFF, 0xFF, 0xFF]])
    );
    assert_eq!(scan(b""), keys(&all_keys));
    // The empty prefix, walked in pages of 2.
    let mut walked = Vec::new();
    let mut resume: Option<Vec<u8>> = None;
    for _ in 0..all_keys.len() {
        let page = engine.scan_prefix(b"", resume.as_deref(), 2).unwrap();
        walked.extend(page_keys(&page));
        resume = page.resume_after;
        if resume.is_none() {
            break;
        }
    }
    assert_eq!(resume, None);
    assert_eq!(walked, keys(&all_keys));
}

fn case_c16<F: KvTestFactory>() {
    let (engine, _guard) = F::make();
    put(&engine, b"p1", b"v");
    let result = engine.scan_prefix(b"", None, 0);
    assert!(
        matches!(result, Err(KvError::InvalidArgument(_))),
        "{result:?}"
    );
    let result = engine.scan_prefix(b"p", Some(b"p0"), 0);
    assert!(
        matches!(result, Err(KvError::InvalidArgument(_))),
        "{result:?}"
    );
}

fn case_c17<F: KvTestFactory>() {
    let (engine, _guard) = F::make();
    for key in [&b"a"[..], b"m1", b"m2", b"m5", b"z"] {
        put(&engine, key, b"old");
    }
    let before = all(&engine);
    let invalid: Vec<(KeyRange, Entries)> = vec![
        // unsorted
        (
            bounded(b"m", b"n"),
            vec![pair(b"m4", b"x"), pair(b"m3", b"x")],
        ),
        // not strictly ascending
        (
            bounded(b"m", b"n"),
            vec![pair(b"m4", b"x"), pair(b"m4", b"y")],
        ),
        // above the range
        (
            bounded(b"m", b"n"),
            vec![pair(b"m3", b"x"), pair(b"z1", b"x")],
        ),
        // the range's end itself
        (bounded(b"m", b"n"), vec![pair(b"n", b"x")]),
        // below the range
        (bounded(b"m", b"n"), vec![pair(b"a1", b"x")]),
        // open below
        (
            KeyRange {
                start: None,
                end: Some(b"n".to_vec()),
            },
            vec![pair(b"m3", b"x")],
        ),
        // open above
        (
            KeyRange {
                start: Some(b"m".to_vec()),
                end: None,
            },
            vec![pair(b"m3", b"x")],
        ),
        // start above end
        (bounded(b"n", b"m"), Entries::new()),
    ];
    if engine.capabilities().ingest {
        engine
            .ingest(
                bounded(b"m", b"n"),
                vec![pair(b"m0", b"new"), pair(b"m3", b"new")],
                NOSYNC,
            )
            .unwrap();
        let replaced = vec![
            pair(b"a", b"old"),
            pair(b"m0", b"new"),
            pair(b"m3", b"new"),
            pair(b"z", b"old"),
        ];
        assert_eq!(all(&engine), replaced);
        for (range, sorted) in invalid {
            let shown = format!("{range:?} {sorted:?}");
            let result = engine.ingest(range, sorted, SYNC);
            assert!(
                matches!(result, Err(KvError::InvalidArgument(_))),
                "{shown}: {result:?}"
            );
            assert_eq!(all(&engine), replaced, "{shown}");
        }
        // An empty input empties the range.
        engine
            .ingest(bounded(b"m", b"n"), Entries::new(), SYNC)
            .unwrap();
        assert_eq!(all(&engine), vec![pair(b"a", b"old"), pair(b"z", b"old")]);
    } else {
        let mut calls = invalid;
        calls.push((
            bounded(b"m", b"n"),
            vec![pair(b"m0", b"new"), pair(b"m3", b"new")],
        ));
        for (range, sorted) in calls {
            let shown = format!("{range:?} {sorted:?}");
            let result = engine.ingest(range, sorted, SYNC);
            assert!(
                matches!(result, Err(KvError::Unsupported(KvCapability::Ingest))),
                "{shown}: {result:?}"
            );
            assert_eq!(all(&engine), before, "{shown}");
        }
    }
}

fn case_c18<F: KvTestFactory>() {
    let (engine, _guard) = F::make();
    let capabilities = engine.capabilities();
    assert!(!capabilities.engine.is_empty());
    let result = engine.ingest(bounded(b"m", b"n"), vec![pair(b"m1", b"v")], NOSYNC);
    if capabilities.ingest {
        assert!(result.is_ok(), "ingest declared present: {result:?}");
        assert_eq!(engine.get(b"m1").unwrap(), Some(b"v".to_vec()));
    } else {
        assert!(
            matches!(result, Err(KvError::Unsupported(KvCapability::Ingest))),
            "ingest declared absent: {result:?}"
        );
        assert_eq!(engine.get(b"m1").unwrap(), None);
    }
    assert_eq!(engine.capabilities(), capabilities);
}

fn case_c19<F: KvTestFactory>() {
    let (engine, _guard) = F::make();
    engine.sync().unwrap();
    put(&engine, b"a", b"1");
    engine.sync().unwrap();
    engine.sync().unwrap();
    let mut batch = WriteBatch::new();
    batch.put(b"b", b"1");
    engine.write(batch, SYNC).unwrap();
    engine.sync().unwrap();
    assert_eq!(all(&engine), vec![pair(b"a", b"1"), pair(b"b", b"1")]);
}

fn case_c20<F: KvTestFactory>() {
    let (engine, _guard) = F::make();
    put(&engine, b"a", b"1");
    let capabilities = engine.capabilities();
    engine.shutdown().unwrap();
    let shut = |r: &str, ok: bool| assert!(ok, "{r} after shutdown");
    let result = engine.get(b"a");
    shut(
        &format!("get {result:?}"),
        matches!(result, Err(KvError::ShutDown)),
    );
    let mut batch = WriteBatch::new();
    batch.put(b"b", b"1");
    let result = engine.write(batch, SYNC);
    shut(
        &format!("write {result:?}"),
        matches!(result, Err(KvError::ShutDown)),
    );
    let result = engine.delete(b"a", NOSYNC);
    shut(
        &format!("delete {result:?}"),
        matches!(result, Err(KvError::ShutDown)),
    );
    let result = engine.delete_range(b"a", b"z", NOSYNC);
    shut(
        &format!("delete_range {result:?}"),
        matches!(result, Err(KvError::ShutDown)),
    );
    let result = engine.snapshot();
    shut("snapshot", matches!(result, Err(KvError::ShutDown)));
    drop(result);
    let result = engine.scan_prefix(b"", None, 10);
    shut(
        &format!("scan_prefix {result:?}"),
        matches!(result, Err(KvError::ShutDown)),
    );
    let result = engine.ingest(bounded(b"m", b"n"), vec![pair(b"m1", b"v")], NOSYNC);
    shut(
        &format!("ingest {result:?}"),
        matches!(result, Err(KvError::ShutDown)),
    );
    let result = engine.sync();
    shut(
        &format!("sync {result:?}"),
        matches!(result, Err(KvError::ShutDown)),
    );
    let result = engine.shutdown();
    shut(
        &format!("shutdown {result:?}"),
        matches!(result, Err(KvError::ShutDown)),
    );
    assert_eq!(engine.capabilities(), capabilities);
}

fn case_c21<F: KvTestFactory>() {
    let (engine, _guard) = F::make();
    let every_byte: Vec<u8> = (0..=255u8).collect();
    let large: Vec<u8> = (0..1024 * 1024)
        .map(|i: u32| i.to_le_bytes()[0] ^ (i >> 8).to_le_bytes()[0])
        .collect();
    let values: [(&[u8], &[u8]); 3] =
        [(b"empty", b""), (b"every", &every_byte), (b"large", &large)];
    for (key, value) in values {
        let mut batch = WriteBatch::new();
        batch.put(key, value);
        engine.write(batch, SYNC).unwrap();
    }
    for (key, value) in values {
        assert_eq!(engine.get(key).unwrap().as_deref(), Some(value));
    }
    let expected: Entries = vec![
        pair(b"empty", b""),
        pair(b"every", &every_byte),
        pair(b"large", &large),
    ];
    assert!(
        all(&engine) == expected,
        "iteration differs from the written values"
    );
}

fn case_c22<F: KvTestFactory>() {
    let (engine, _guard) = F::make();
    let mut model = Model::new();
    for chunk in 0..20u32 {
        let mut batch = WriteBatch::new();
        for i in chunk * 500..(chunk + 1) * 500 {
            let key = format!("key{i:05}").into_bytes();
            batch.put(key.clone(), i.to_be_bytes());
            model.insert(key, i.to_be_bytes().to_vec());
        }
        engine.write(batch, NOSYNC).unwrap();
    }
    let got = all(&engine);
    assert_eq!(got.len(), 10_000);
    assert!(got == entries(&model), "iteration differs from the model");
    let page = engine.scan_prefix(b"key", None, 10_001).unwrap();
    assert_eq!(page.entries.len(), 10_000);
    assert_eq!(page.resume_after, None);
}

fn case_c23<F: KvTestFactory>() {
    let (engine, _guard) = F::make();
    let mut batch = WriteBatch::new();
    batch.put(b"a", b"1");
    engine.write(batch, NOSYNC).unwrap();
    assert_eq!(engine.get(b"a").unwrap(), Some(b"1".to_vec()));
    let snapshot = engine.snapshot().unwrap();
    assert_eq!(snapshot.get(b"a").unwrap(), Some(b"1".to_vec()));
    drop(snapshot);
    let page = engine.scan_prefix(b"a", None, 10).unwrap();
    assert_eq!(page.entries, vec![pair(b"a", b"1")]);
    engine.delete(b"a", NOSYNC).unwrap();
    assert_eq!(engine.get(b"a").unwrap(), None);
    assert_eq!(all(&engine), Entries::new());
}

/// One step of the model test.
#[derive(Debug, Clone)]
enum Step {
    Put(Vec<u8>, Vec<u8>, bool),
    Delete(Vec<u8>, bool),
    DeleteRange(Vec<u8>, Vec<u8>, bool),
    Batch(Vec<BatchOp>, bool),
    /// Take a snapshot, apply the batch, then iterate the snapshot (whole and over a range) and the engine.
    SnapshotMutateIterate(Vec<BatchOp>, Option<Vec<u8>>, Option<Vec<u8>>),
    /// A `scan_prefix` walk; `between[i]` is written after page `i`.
    Walk {
        prefix: Vec<u8>,
        limit: usize,
        between: Vec<Vec<Mutation>>,
    },
    Get(Vec<u8>),
    Sync,
}

/// A write between the pages of a walk.
#[derive(Debug, Clone)]
enum Mutation {
    Put(Vec<u8>, Vec<u8>),
    Delete(Vec<u8>),
    /// Delete the key the walk will resume after.
    DeleteCursor,
}

/// Keys over a 4-letter alphabet, 0 to 4 long, so prefixes are dense.
fn key_strategy() -> impl Strategy<Value = Vec<u8>> {
    proptest::collection::vec(0u8..4, 0..=4).prop_map(|v| v.into_iter().map(|b| b'a' + b).collect())
}

fn value_strategy() -> impl Strategy<Value = Vec<u8>> {
    proptest::collection::vec(any::<u8>(), 0..4)
}

fn op_strategy() -> impl Strategy<Value = BatchOp> {
    prop_oneof![
        3 => (key_strategy(), value_strategy()).prop_map(|(key, value)| BatchOp::Put { key, value }),
        2 => key_strategy().prop_map(|key| BatchOp::Delete { key }),
        1 => (key_strategy(), key_strategy()).prop_map(|(start, end)| BatchOp::DeleteRange { start, end }),
    ]
}

fn mutation_strategy() -> impl Strategy<Value = Mutation> {
    prop_oneof![
        2 => (key_strategy(), value_strategy()).prop_map(|(k, v)| Mutation::Put(k, v)),
        1 => key_strategy().prop_map(Mutation::Delete),
        1 => Just(Mutation::DeleteCursor),
    ]
}

fn step_strategy() -> impl Strategy<Value = Step> {
    prop_oneof![
        6 => (key_strategy(), value_strategy(), any::<bool>()).prop_map(|(k, v, s)| Step::Put(k, v, s)),
        3 => (key_strategy(), any::<bool>()).prop_map(|(k, s)| Step::Delete(k, s)),
        2 => (key_strategy(), key_strategy(), any::<bool>()).prop_map(|(a, b, s)| Step::DeleteRange(a, b, s)),
        3 => (proptest::collection::vec(op_strategy(), 1..=8), any::<bool>()).prop_map(|(ops, s)| Step::Batch(ops, s)),
        2 => (
            proptest::collection::vec(op_strategy(), 1..=8),
            proptest::option::of(key_strategy()),
            proptest::option::of(key_strategy()),
        )
            .prop_map(|(ops, a, b)| Step::SnapshotMutateIterate(ops, a, b)),
        2 => (
            proptest::collection::vec(0u8..4, 0..=2).prop_map(|v| v.into_iter().map(|b| b'a' + b).collect()),
            1usize..=5,
            proptest::collection::vec(proptest::collection::vec(mutation_strategy(), 0..=3), 0..40),
        )
            .prop_map(|(prefix, limit, between)| Step::Walk { prefix, limit, between }),
        2 => key_strategy().prop_map(Step::Get),
        1 => Just(Step::Sync),
    ]
}

fn script_strategy() -> impl Strategy<Value = Vec<Step>> {
    proptest::collection::vec(step_strategy(), 0..=64)
}

/// Writes `ops` to both the engine and the model; an invalid batch must be refused whole.
fn write_both<E: KvEngine>(engine: &E, model: &mut Model, ops: &[BatchOp], sync: bool) {
    let result = engine.write(batch_of(ops), WriteOptions { sync });
    if batch_is_valid(ops) {
        result.unwrap();
        apply(model, ops);
    } else {
        assert!(
            matches!(result, Err(KvError::InvalidArgument(_))),
            "{ops:?}: {result:?}"
        );
    }
}

fn case_c24<F: KvTestFactory>(script: Vec<Step>) {
    let (engine, _guard) = F::make();
    let mut model = Model::new();
    for step in script {
        match step {
            Step::Put(key, value, sync) => {
                write_both(&engine, &mut model, &[BatchOp::Put { key, value }], sync);
            }
            Step::Delete(key, sync) => {
                engine.delete(&key, WriteOptions { sync }).unwrap();
                model.remove(&key);
            }
            Step::DeleteRange(start, end, sync) => {
                let result = engine.delete_range(&start, &end, WriteOptions { sync });
                if start <= end {
                    result.unwrap();
                    apply(&mut model, &[BatchOp::DeleteRange { start, end }]);
                } else {
                    assert!(
                        matches!(result, Err(KvError::InvalidArgument(_))),
                        "{result:?}"
                    );
                }
            }
            Step::Batch(ops, sync) => write_both(&engine, &mut model, &ops, sync),
            Step::SnapshotMutateIterate(ops, a, b) => {
                let before = model.clone();
                let snapshot = engine.snapshot().unwrap();
                write_both(&engine, &mut model, &ops, false);
                assert_eq!(
                    snapshot_entries(snapshot.as_ref(), KeyRange::default()),
                    entries(&before)
                );
                let (start, end) = match (a, b) {
                    (Some(a), Some(b)) if a > b => (Some(b), Some(a)),
                    other => other,
                };
                let range = KeyRange {
                    start: start.clone(),
                    end: end.clone(),
                };
                let expected: Entries = before
                    .iter()
                    .filter(|(k, _)| {
                        start.as_ref().is_none_or(|s| *k >= s)
                            && end.as_ref().is_none_or(|e| *k < e)
                    })
                    .map(|(k, v)| (k.clone(), v.clone()))
                    .collect();
                assert_eq!(snapshot_entries(snapshot.as_ref(), range), expected);
            }
            Step::Walk {
                prefix,
                limit,
                between,
            } => {
                let mut resume: Option<Vec<u8>> = None;
                let mut page_no = 0;
                loop {
                    let page = engine
                        .scan_prefix(&prefix, resume.as_deref(), limit)
                        .unwrap();
                    assert_eq!(page, model_page(&model, &prefix, resume.as_deref(), limit));
                    let Some(next) = page.resume_after else { break };
                    page_no += 1;
                    assert!(page_no <= 2_000, "the walk does not end");
                    for mutation in between.get(page_no - 1).cloned().unwrap_or_default() {
                        let ops = match mutation {
                            Mutation::Put(key, value) => vec![BatchOp::Put { key, value }],
                            Mutation::Delete(key) => vec![BatchOp::Delete { key }],
                            Mutation::DeleteCursor => vec![BatchOp::Delete { key: next.clone() }],
                        };
                        write_both(&engine, &mut model, &ops, false);
                    }
                    resume = Some(next);
                }
            }
            Step::Get(key) => assert_eq!(engine.get(&key).unwrap(), model.get(&key).cloned()),
            Step::Sync => engine.sync().unwrap(),
        }
        assert_eq!(all(&engine), entries(&model));
    }
}

fn case_c25<F: KvTestFactory>() {
    const KEYS: usize = 16;
    const BATCHES: u32 = 2_000;
    let (engine, _guard) = F::make();
    let names: Vec<Vec<u8>> = (0..KEYS).map(|i| format!("k{i:02}").into_bytes()).collect();
    let mut batch = WriteBatch::new();
    for name in &names {
        batch.put(name.clone(), 0u32.to_be_bytes());
    }
    engine.write(batch, NOSYNC).unwrap();
    let done = AtomicBool::new(false);
    let barrier = std::sync::Barrier::new(2);
    let loops = std::thread::scope(|s| {
        let writer = s.spawn(|| {
            barrier.wait();
            for i in 1..=BATCHES {
                let mut batch = WriteBatch::new();
                for name in &names {
                    batch.put(name.clone(), i.to_be_bytes());
                }
                engine.write(batch, NOSYNC).unwrap();
            }
            done.store(true, Ordering::SeqCst);
        });
        let reader = s.spawn(|| {
            barrier.wait();
            let mut loops = 0u64;
            let mut previous = 0u32;
            let one_value = |values: &[Vec<u8>], view: &str| -> u32 {
                assert_eq!(values.len(), KEYS, "{view}: a key is missing");
                assert!(
                    values.iter().all(|v| *v == values[0]),
                    "{view}: part of a batch is visible: {values:?}"
                );
                u32::from_be_bytes(values[0].as_slice().try_into().unwrap())
            };
            while !done.load(Ordering::SeqCst) {
                let snapshot = engine.snapshot().unwrap();
                let got: Vec<Vec<u8>> = names
                    .iter()
                    .filter_map(|n| snapshot.get(n).unwrap())
                    .collect();
                let value = one_value(&got, "get");
                let iterated: Vec<Vec<u8>> =
                    snapshot_entries(snapshot.as_ref(), KeyRange::default())
                        .into_iter()
                        .map(|(_, v)| v)
                        .collect();
                assert_eq!(
                    one_value(&iterated, "iter"),
                    value,
                    "iter and get of one snapshot differ"
                );
                drop(snapshot);
                let page = engine.scan_prefix(b"k", None, KEYS).unwrap();
                let scanned: Vec<Vec<u8>> = page.entries.into_iter().map(|(_, v)| v).collect();
                one_value(&scanned, "scan_prefix");
                assert!(
                    value >= previous,
                    "a later snapshot lost a batch: {value} < {previous}"
                );
                previous = value;
                loops += 1;
            }
            loops
        });
        writer.join().unwrap();
        reader.join().unwrap()
    });
    for name in &names {
        assert_eq!(
            engine.get(name).unwrap(),
            Some(BATCHES.to_be_bytes().to_vec())
        );
    }
    eprintln!("c25 reader loops: {loops}");
}

fn case_c26<F: KvTestFactory>() {
    let (engine, _guard) = F::make();
    let mut batch = WriteBatch::new();
    batch.put(b"o", b"v");
    batch.put(b"q", b"v");
    for i in 0..20 {
        batch.put(format!("p{i:02}"), b"v");
    }
    engine.write(batch, NOSYNC).unwrap();
    let names =
        |list: &[&str]| -> Vec<Vec<u8>> { list.iter().map(|k| k.as_bytes().to_vec()).collect() };
    let expected: [(&[&str], Option<&str>); 8] = [
        (&["p00", "p01", "p02"], Some("p02")),
        (&["p025", "p03", "p04"], Some("p04")),
        (&["p05", "p07", "p08"], Some("p08")),
        (&["p09", "p10", "p11"], Some("p11")),
        (&["p12", "p13", "p14"], Some("p14")),
        (&["p15", "p16", "p17"], Some("p17")),
        (&["p18", "p19", "p19x"], Some("p19x")),
        (&[], None),
    ];
    let mut returned = Vec::new();
    let mut resume: Option<Vec<u8>> = None;
    for (call, (page_keys_expected, resume_expected)) in expected.iter().enumerate() {
        let page = engine.scan_prefix(b"p", resume.as_deref(), 3).unwrap();
        assert_eq!(
            page_keys(&page),
            names(page_keys_expected),
            "page {}",
            call + 1
        );
        assert_eq!(
            page.resume_after,
            resume_expected.map(|k| k.as_bytes().to_vec()),
            "page {}",
            call + 1
        );
        returned.extend(page_keys(&page));
        match call {
            0 => {
                engine.delete(b"p02", NOSYNC).unwrap();
                put(&engine, b"p015", b"v");
                put(&engine, b"p025", b"v");
            }
            1 => {
                engine.delete(b"p06", NOSYNC).unwrap();
                put(&engine, b"p005", b"v");
            }
            2 => {
                engine.delete(b"p08", NOSYNC).unwrap();
                put(&engine, b"p19x", b"v");
            }
            _ => {}
        }
        resume = page.resume_after;
    }
    assert_eq!(resume, None);
    for never in ["o", "q", "p005", "p015", "p06"] {
        assert!(
            !returned.contains(&never.as_bytes().to_vec()),
            "{never} was returned"
        );
    }
}

fn case_c27<F: KvReopenFactory>() {
    let (engine, guard) = F::make();
    let mut model = Model::new();
    for i in 1..=50u32 {
        let mut ops = vec![BatchOp::Put {
            key: format!("r{i:02}").into_bytes(),
            value: i.to_be_bytes().to_vec(),
        }];
        if i % 3 == 0 {
            ops.push(BatchOp::Delete {
                key: format!("r{:02}", i - 2).into_bytes(),
            });
        }
        if i == 30 {
            ops.push(BatchOp::DeleteRange {
                start: b"r05".to_vec(),
                end: b"r10".to_vec(),
            });
        }
        engine
            .write(batch_of(&ops), WriteOptions { sync: i == 25 })
            .unwrap();
        apply(&mut model, &ops);
    }
    assert_eq!(all(&engine), entries(&model));
    engine.shutdown().unwrap();
    drop(engine);
    let reopened = F::reopen(&guard);
    assert_eq!(all(&reopened), entries(&model));
    reopened.shutdown().unwrap();
}

/// Emits the conformance tests for one engine.
///
/// `full` runs every shared case; `file` runs the cases where a real file
/// matters (many keys) and the reopen case, and needs a [`KvReopenFactory`].
macro_rules! kv_engine_conformance {
    (full $module:ident, $factory:ty) => {
        mod $module {
            use super::*;

            #[test]
            fn c01_get_of_an_absent_key_is_none() {
                case_c01::<$factory>();
            }
            #[test]
            fn c02_a_put_is_read_back_and_an_overwrite_wins() {
                case_c02::<$factory>();
            }
            #[test]
            fn c03_a_delete_removes_and_a_delete_of_an_absent_key_is_ok() {
                case_c03::<$factory>();
            }
            #[test]
            fn c04_batch_ops_apply_in_order() {
                case_c04::<$factory>();
            }
            #[test]
            fn c05_a_batch_with_an_invalid_range_changes_nothing() {
                case_c05::<$factory>();
            }
            #[test]
            fn c06_an_empty_batch_is_ok_and_changes_nothing() {
                case_c06::<$factory>();
            }
            #[test]
            fn c07_delete_range_is_half_open() {
                case_c07::<$factory>();
            }
            #[test]
            fn c08_delete_range_with_equal_bounds_changes_nothing() {
                case_c08::<$factory>();
            }
            #[test]
            fn c09_iteration_is_in_ascending_byte_order() {
                case_c09::<$factory>();
            }
            #[test]
            fn c10_iteration_honours_inclusive_exclusive_and_open_bounds() {
                case_c10::<$factory>();
            }
            #[test]
            fn c11_a_snapshot_does_not_see_writes_made_after_it() {
                case_c11::<$factory>();
            }
            #[test]
            fn c12_an_iterator_is_unaffected_by_writes_made_while_it_runs() {
                case_c12::<$factory>();
            }
            #[test]
            fn c13_scan_prefix_returns_exactly_the_prefix_keys_in_order() {
                case_c13::<$factory>();
            }
            #[test]
            fn c14_scan_prefix_pages_cover_every_key_once() {
                case_c14::<$factory>();
            }
            #[test]
            fn c15_scan_prefix_of_a_prefix_ending_in_ff_and_the_empty_prefix() {
                case_c15::<$factory>();
            }
            #[test]
            fn c16_scan_prefix_with_limit_zero_is_invalid() {
                case_c16::<$factory>();
            }
            #[test]
            fn c17_ingest_replaces_a_range_or_is_refused_as_unsupported() {
                case_c17::<$factory>();
            }
            #[test]
            fn c18_capabilities_are_truthful() {
                case_c18::<$factory>();
            }
            #[test]
            fn c19_sync_is_ok_and_repeatable() {
                case_c19::<$factory>();
            }
            #[test]
            fn c20_after_shutdown_every_call_fails_with_shut_down() {
                case_c20::<$factory>();
            }
            #[test]
            fn c21_values_round_trip_byte_for_byte() {
                case_c21::<$factory>();
            }
            #[test]
            fn c22_ten_thousand_keys_iterate_in_order_and_count() {
                case_c22::<$factory>();
            }
            #[test]
            fn c23_unsynced_writes_are_visible_at_once() {
                case_c23::<$factory>();
            }
            proptest! {
                #![proptest_config(ProptestConfig::with_cases(256))]
                #[test]
                fn c24_the_engine_matches_the_model(script in script_strategy()) {
                    case_c24::<$factory>(script);
                }
            }
            #[test]
            fn c25_a_concurrent_reader_never_sees_part_of_a_batch() {
                case_c25::<$factory>();
            }
            #[test]
            fn c26_a_cursor_walk_with_writes_between_pages_follows_the_page_rule() {
                case_c26::<$factory>();
            }
        }
    };
    (file $module:ident, $factory:ty) => {
        mod $module {
            use super::*;

            #[test]
            fn c09_iteration_is_in_ascending_byte_order() {
                case_c09::<$factory>();
            }
            #[test]
            fn c14_scan_prefix_pages_cover_every_key_once() {
                case_c14::<$factory>();
            }
            #[test]
            fn c22_ten_thousand_keys_iterate_in_order_and_count() {
                case_c22::<$factory>();
            }
            #[test]
            fn c27_shutdown_then_reopen_keeps_every_returned_batch() {
                case_c27::<$factory>();
            }
        }
    };
}

kv_engine_conformance!(full mem, MemFactory);
#[cfg(feature = "redb")]
kv_engine_conformance!(full redb, RedbMemFactory);
#[cfg(feature = "redb")]
kv_engine_conformance!(file redb_file, RedbFileFactory);

// ---- the reference engine's crash hooks ---------------------------------------

mod mem_specific {
    use super::*;

    fn ops(list: &[(&str, Option<&str>)]) -> Vec<BatchOp> {
        list.iter()
            .map(|(key, value)| match value {
                Some(value) => BatchOp::Put {
                    key: key.as_bytes().to_vec(),
                    value: value.as_bytes().to_vec(),
                },
                None => BatchOp::Delete {
                    key: key.as_bytes().to_vec(),
                },
            })
            .collect()
    }

    #[test]
    fn m01_the_mem_crash_image_keeps_durable_and_exactly_the_first_keep_unsynced_batches() {
        let engine = MemKvEngine::new();
        let b1 = ops(&[
            ("a", Some("1")),
            ("b", Some("1")),
            ("c", Some("1")),
            ("d", Some("1")),
        ]);
        // The first op of each unsynced batch puts a key no earlier batch has, so
        // applying that op alone changes the visible state.
        let b2 = ops(&[("e", Some("2")), ("a", Some("2"))]);
        let mut b3 = ops(&[("f", Some("3")), ("a", None)]);
        b3.push(BatchOp::DeleteRange {
            start: b"b".to_vec(),
            end: b"d".to_vec(),
        });
        let b4 = ops(&[("g", Some("4")), ("d", Some("4"))]);
        engine.write(batch_of(&b1), SYNC).unwrap();
        for batch in [&b2, &b3, &b4] {
            engine.write(batch_of(batch), NOSYNC).unwrap();
        }
        let model_of = |batches: &[&Vec<BatchOp>]| -> Entries {
            let mut model = Model::new();
            for batch in batches {
                apply(&mut model, batch);
            }
            entries(&model)
        };
        let unsynced = [&b2, &b3, &b4];
        for keep in 0..=5usize {
            let image = engine.crash_image(keep);
            let mut expected_batches = vec![&b1];
            expected_batches.extend(unsynced.iter().take(keep.min(3)));
            let expected = model_of(&expected_batches);
            assert_eq!(all(&image), expected, "keep {keep}");
            assert_eq!(image.calls_so_far(), 0, "keep {keep}");
            // The image's durable state equals its live state.
            assert_eq!(
                all(&image.crash_image(0)),
                expected,
                "keep {keep}: durable of the image"
            );
            assert_eq!(all(&engine), model_of(&[&b1, &b2, &b3, &b4]));
        }
        engine.sync().unwrap();
        assert_eq!(all(&engine.crash_image(0)), model_of(&[&b1, &b2, &b3, &b4]));
        let b5 = ops(&[("h", Some("5"))]);
        let b6 = ops(&[("i", Some("6"))]);
        engine.write(batch_of(&b5), NOSYNC).unwrap();
        engine.write(batch_of(&b6), SYNC).unwrap();
        assert_eq!(
            all(&engine.crash_image(0)),
            model_of(&[&b1, &b2, &b3, &b4, &b5, &b6])
        );
    }

    #[test]
    fn m02_a_dead_mem_engine_fails_the_armed_call_and_every_later_call_and_applies_nothing() {
        let engine = MemKvEngine::new();
        assert_eq!(engine.calls_so_far(), 0);
        engine.get(b"x").unwrap();
        drop(engine.snapshot().unwrap());
        engine.scan_prefix(b"", None, 10).unwrap();
        assert_eq!(engine.calls_so_far(), 0, "reads are not counted");
        put(&engine, b"x", b"v");
        assert_eq!(engine.calls_so_far(), 1);
        engine.delete(b"x", NOSYNC).unwrap();
        assert_eq!(engine.calls_so_far(), 2);
        engine.delete_range(b"a", b"b", NOSYNC).unwrap();
        assert_eq!(engine.calls_so_far(), 3);
        let mut invalid = WriteBatch::new();
        invalid.delete_range(b"z", b"a");
        assert!(matches!(
            engine.write(invalid, NOSYNC),
            Err(KvError::InvalidArgument(_))
        ));
        assert_eq!(engine.calls_so_far(), 4, "an invalid write is counted");
        engine.sync().unwrap();
        assert_eq!(engine.calls_so_far(), 5);
        engine
            .ingest(bounded(b"m", b"n"), vec![pair(b"m1", b"v")], NOSYNC)
            .unwrap();
        assert_eq!(engine.calls_so_far(), 6);

        engine.arm_death(7);
        put(&engine, b"w6", b"v");
        assert_eq!(engine.calls_so_far(), 7);
        let mut batch = WriteBatch::new();
        batch.put(b"w7", b"v");
        let result = engine.write(batch, NOSYNC);
        assert!(
            matches!(result, Err(KvError::Other(_))),
            "the armed call: {result:?}"
        );
        let image = engine.crash_image(usize::MAX);
        assert_eq!(image.get(b"w7").unwrap(), None);

        let result = engine.get(b"m1");
        assert!(matches!(result, Err(KvError::Other(_))), "get: {result:?}");
        assert!(
            matches!(engine.snapshot(), Err(KvError::Other(_))),
            "snapshot"
        );
        let result = engine.scan_prefix(b"", None, 10);
        assert!(
            matches!(result, Err(KvError::Other(_))),
            "scan_prefix: {result:?}"
        );
        let mut batch = WriteBatch::new();
        batch.put(b"w8", b"v");
        let result = engine.write(batch, SYNC);
        assert!(
            matches!(result, Err(KvError::Other(_))),
            "write: {result:?}"
        );
        let result = engine.sync();
        assert!(matches!(result, Err(KvError::Other(_))), "sync: {result:?}");
        let result = engine.shutdown();
        assert!(
            matches!(result, Err(KvError::Other(_))),
            "shutdown: {result:?}"
        );
        assert_eq!(engine.capabilities().engine, "mem");
        assert_eq!(
            all(&engine.crash_image(usize::MAX)),
            vec![pair(b"m1", b"v"), pair(b"w6", b"v")]
        );

        let fresh = MemKvEngine::new();
        fresh.arm_death(0);
        let mut batch = WriteBatch::new();
        batch.put(b"a", b"v");
        let result = fresh.write(batch, SYNC);
        assert!(matches!(result, Err(KvError::Other(_))), "{result:?}");
        assert_eq!(all(&fresh.crash_image(usize::MAX)), Entries::new());
    }
}

// ---- redb ---------------------------------------------------------------------

#[cfg(feature = "redb")]
mod redb_specific {
    use std::io;
    use std::panic::{catch_unwind, AssertUnwindSafe};
    use std::sync::mpsc::{channel, Receiver, Sender};
    use std::sync::{Arc, Mutex};

    use ::redb::backends::InMemoryBackend;
    use ::redb::StorageBackend;

    use super::super::redb_engine::{
        kv_from_commit, kv_from_storage, kv_from_table, kv_from_transaction,
    };
    use super::super::RedbKvEngine;
    use super::*;
    use crate::storage::StoreBrokenCause;

    // The test's backend is nested in it, as is the checking code it parks for.
    #[allow(clippy::too_many_lines)]
    #[test]
    fn r01_a_reader_never_sees_part_of_a_batch_while_its_commit_syncs() {
        enum Msg {
            Parked,
            Done,
        }

        /// Parks every `sync_data` while armed, until the checking code releases it.
        struct Parking {
            inner: InMemoryBackend,
            armed: Arc<AtomicBool>,
            parked: Mutex<Sender<Msg>>,
            release: Mutex<Receiver<()>>,
        }

        impl std::fmt::Debug for Parking {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("Parking")
            }
        }

        impl StorageBackend for Parking {
            fn len(&self) -> io::Result<u64> {
                self.inner.len()
            }
            fn read(&self, offset: u64, len: usize) -> io::Result<Vec<u8>> {
                self.inner.read(offset, len)
            }
            fn set_len(&self, len: u64) -> io::Result<()> {
                self.inner.set_len(len)
            }
            fn write(&self, offset: u64, data: &[u8]) -> io::Result<()> {
                self.inner.write(offset, data)
            }
            fn sync_data(&self, eventual: bool) -> io::Result<()> {
                if self.armed.load(Ordering::SeqCst) {
                    // A closed channel ends the park with an error instead of
                    // blocking or panicking, so a failed check cannot hang the case.
                    let sent = self
                        .parked
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner)
                        .send(Msg::Parked);
                    if sent.is_err() {
                        return Err(io::Error::other("parked channel closed"));
                    }
                    let release = self
                        .release
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner);
                    if release.recv().is_err() {
                        return Err(io::Error::other("release channel closed"));
                    }
                }
                self.inner.sync_data(eventual)
            }
        }

        fn run(variant_ii: bool) {
            let armed = Arc::new(AtomicBool::new(false));
            let (parked_tx, parked_rx) = channel::<Msg>();
            let (release, release_rx) = channel::<()>();
            let done = parked_tx.clone();
            let engine = RedbKvEngine::with_backend(Parking {
                inner: InMemoryBackend::new(),
                armed: Arc::clone(&armed),
                parked: Mutex::new(parked_tx),
                release: Mutex::new(release_rx),
            })
            .unwrap();
            let mut batch = WriteBatch::new();
            batch.put(b"a", 0u32.to_be_bytes());
            batch.put(b"b", 0u32.to_be_bytes());
            engine.write(batch, SYNC).unwrap();
            if variant_ii {
                // A pending non-durable commit that the batch's durable commit makes durable.
                let mut batch = WriteBatch::new();
                batch.put(b"c", 1u32.to_be_bytes());
                engine.write(batch, NOSYNC).unwrap();
            }
            // Armed only now: opening the engine and the setup writes sync too.
            armed.store(true, Ordering::SeqCst);
            std::thread::scope(|s| {
                // Taken by value: a failed assertion below drops the only release
                // sender, which ends a parked commit instead of leaving it blocked.
                let release = release;
                let parked_rx = parked_rx;
                let engine = &engine;
                let writer = s.spawn(move || {
                    let mut batch = WriteBatch::new();
                    batch.put(b"a", 1u32.to_be_bytes());
                    batch.put(b"b", 1u32.to_be_bytes());
                    let r = catch_unwind(AssertUnwindSafe(|| engine.write(batch, SYNC)));
                    let _ = done.send(Msg::Done);
                    r
                });
                let mut parks = 0u32;
                // Every message before `Done` is a park.
                while let Msg::Parked = parked_rx.recv().unwrap() {
                    let snapshot = engine.snapshot().unwrap();
                    let a = snapshot.get(b"a").unwrap();
                    let b = snapshot.get(b"b").unwrap();
                    assert!(
                        a.is_some() && a == b,
                        "part of a batch is visible: a={a:?} b={b:?}"
                    );
                    let iterated = snapshot_entries(snapshot.as_ref(), KeyRange::default());
                    let find = |key: &[u8]| {
                        iterated
                            .iter()
                            .find(|(k, _)| k == key)
                            .map(|(_, v)| v.clone())
                    };
                    assert_eq!(find(b"a"), a, "iteration and get differ");
                    assert_eq!(find(b"b"), b, "iteration and get differ");
                    drop(snapshot);
                    parks += 1;
                    release.send(()).unwrap();
                }
                let r = writer.join().unwrap();
                assert!(matches!(r, Ok(Ok(()))), "the writer failed");
                armed.store(false, Ordering::SeqCst);
                assert!(parks >= 1, "the durable commit never synced");
            });
            assert_eq!(engine.get(b"a").unwrap(), Some(1u32.to_be_bytes().to_vec()));
            assert_eq!(engine.get(b"b").unwrap(), Some(1u32.to_be_bytes().to_vec()));
        }

        run(false);
        run(true);
    }

    #[test]
    fn r02_a_latched_engine_reports_broken_previous_io() {
        /// Answers ENOSPC to every write, `set_len` and `sync_data` while `full` is set.
        #[derive(Debug)]
        struct Full {
            inner: InMemoryBackend,
            full: Arc<AtomicBool>,
        }

        impl Full {
            fn room(&self) -> io::Result<()> {
                if self.full.load(Ordering::SeqCst) {
                    Err(io::Error::from_raw_os_error(28))
                } else {
                    Ok(())
                }
            }
        }

        impl StorageBackend for Full {
            fn len(&self) -> io::Result<u64> {
                self.inner.len()
            }
            fn read(&self, offset: u64, len: usize) -> io::Result<Vec<u8>> {
                self.inner.read(offset, len)
            }
            fn set_len(&self, len: u64) -> io::Result<()> {
                self.room()?;
                self.inner.set_len(len)
            }
            fn write(&self, offset: u64, data: &[u8]) -> io::Result<()> {
                self.room()?;
                self.inner.write(offset, data)
            }
            fn sync_data(&self, eventual: bool) -> io::Result<()> {
                self.room()?;
                self.inner.sync_data(eventual)
            }
        }

        let full = Arc::new(AtomicBool::new(false));
        let engine = RedbKvEngine::with_backend(Full {
            inner: InMemoryBackend::new(),
            full: Arc::clone(&full),
        })
        .unwrap();
        put(&engine, b"before", b"v");
        full.store(true, Ordering::SeqCst);
        let mut batch = WriteBatch::new();
        batch.put(b"a", b"v");
        let result = engine.write(batch, SYNC);
        assert!(
            matches!(
                result,
                Err(KvError::Broken {
                    cause: StoreBrokenCause::Io(io::ErrorKind::StorageFull),
                    ..
                })
            ),
            "{result:?}"
        );
        full.store(false, Ordering::SeqCst);
        let mut batch = WriteBatch::new();
        batch.put(b"b", b"v");
        let result = engine.write(batch, SYNC);
        assert!(
            matches!(
                result,
                Err(KvError::Broken {
                    cause: StoreBrokenCause::PreviousIo,
                    ..
                })
            ),
            "{result:?}"
        );
    }

    /// The cause of a `Broken` error, `None` for `Other`; any other variant fails the case.
    fn cause(err: &KvError) -> Option<StoreBrokenCause> {
        match err {
            KvError::Broken { cause, .. } => Some(*cause),
            KvError::Other(_) => None,
            other => panic!("expected Broken or Other, got {other:?}"),
        }
    }

    #[test]
    fn r03_storage_errors_map_to_their_broken_cause() {
        type MakeError = fn() -> ::redb::StorageError;
        let variants: Vec<(MakeError, Option<StoreBrokenCause>)> = vec![
            (
                || ::redb::StorageError::Io(io::Error::from_raw_os_error(28)),
                Some(StoreBrokenCause::Io(io::ErrorKind::StorageFull)),
            ),
            (
                || ::redb::StorageError::PreviousIo,
                Some(StoreBrokenCause::PreviousIo),
            ),
            (
                || ::redb::StorageError::Corrupted("x".to_string()),
                Some(StoreBrokenCause::Corrupted),
            ),
            (
                || ::redb::StorageError::LockPoisoned(std::panic::Location::caller()),
                Some(StoreBrokenCause::LockPoisoned),
            ),
            (|| ::redb::StorageError::ValueTooLarge(1), None),
        ];
        for (make, expected) in variants {
            let shown = make().to_string();
            assert_eq!(cause(&kv_from_storage(make())), expected, "bare {shown}");
            assert_eq!(
                cause(&kv_from_commit(::redb::CommitError::Storage(make()))),
                expected,
                "commit {shown}"
            );
            assert_eq!(
                cause(&kv_from_transaction(::redb::TransactionError::Storage(
                    make()
                ))),
                expected,
                "transaction {shown}"
            );
            assert_eq!(
                cause(&kv_from_table(::redb::TableError::Storage(make()))),
                expected,
                "table {shown}"
            );
        }
        let mismatch = ::redb::TableError::TableTypeMismatch {
            table: "t".to_string(),
            key: ::redb::TypeName::new("k"),
            value: ::redb::TypeName::new("v"),
        };
        assert_eq!(cause(&kv_from_table(mismatch)), None);
    }

    #[test]
    fn r04_redb_ingest_is_refused_as_unsupported_and_never_panics() {
        let engine = RedbKvEngine::with_backend(InMemoryBackend::new()).unwrap();
        let three = vec![pair(b"a", b"1"), pair(b"m5", b"2"), pair(b"z", b"3")];
        for (key, value) in &three {
            put(&engine, key, value);
        }
        assert!(!engine.capabilities().ingest);
        let calls: Vec<(&str, KeyRange, Entries)> = vec![
            (
                "valid",
                bounded(b"m", b"n"),
                vec![pair(b"m1", b"v"), pair(b"m2", b"v")],
            ),
            (
                "open-bounded",
                KeyRange {
                    start: None,
                    end: Some(b"n".to_vec()),
                },
                vec![pair(b"m1", b"v")],
            ),
            (
                "unsorted",
                bounded(b"m", b"n"),
                vec![pair(b"m2", b"v"), pair(b"m1", b"v")],
            ),
            ("outside", bounded(b"m", b"n"), vec![pair(b"z9", b"v")]),
        ];
        for (name, range, sorted) in calls {
            let outcome = catch_unwind(AssertUnwindSafe(|| engine.ingest(range, sorted, SYNC)));
            match outcome {
                Ok(Err(e)) => assert!(
                    matches!(e, KvError::Unsupported(KvCapability::Ingest)),
                    "{name}: {e:?}"
                ),
                Ok(Ok(())) => panic!("{name}: ingest succeeded"),
                Err(payload) => panic!("{name}: ingest panicked: {payload:?}"),
            }
        }
        assert_eq!(all(&engine), three);
    }
}
