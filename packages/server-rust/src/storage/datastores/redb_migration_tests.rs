//! Main tests of the data-directory store: where it lives, the location
//! rules, the one-time resumable migration of a table-per-map file, and the
//! stub at the old path (`TG-ENG-007`, `TG-NAME-001`).
//!
//! Fixtures are legacy files written with raw redb under `table_name_for`;
//! every engine file a fixture holds is written by an engine that is then
//! dropped cleanly. Every wait on a child process or a store has a deadline.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};
use tempfile::{tempdir, TempDir};
use topgun_core::hlc::Timestamp;
use topgun_core::types::Value;

use super::*;
use crate::storage::datastores::engine_store::{
    meta_key, RecordOp, FORMAT_VERSION, META_ACK_MODE, META_FORMAT, META_MIGRATION,
    META_NEXT_MAP_ID,
};
use crate::storage::datastores::kv_crash::{CrashMode, CrashableBackend};
use crate::storage::datastores::kv_engine::{KeyRange, KvEngine, KvError, WriteOptions};
use crate::storage::record::OrMapEntry;

// ---------------------------------------------------------------------------
// Constants and paths
// ---------------------------------------------------------------------------

/// The stub's first line.
const MARKER_LINE: &[u8] = b"TOPGUN-STORE-MOVED 1\n";
/// redb's file magic, as redb 2.6.3 writes it (`tree_store/page_store/header.rs`).
const REDB_FILE_MAGIC: [u8; 9] = *b"redb\x1A\x0A\xA9\x0D\x0A";
/// The largest file the stub may be.
const STUB_LIMIT: u64 = 16_384;
/// 64 MiB, the fixed part of the free-space requirement.
const MIGRATION_SLACK: u64 = 64 * 1024 * 1024;
/// The longest a child run of m05 may take before it counts as a hang.
const CHILD_DEADLINE: Duration = Duration::from_secs(300);

/// The stub's exact bytes for a store in `data_dir`.
fn stub_text(data_dir: &Path) -> Vec<u8> {
    format!(
        "TOPGUN-STORE-MOVED 1\n\
         This is not a database. The TopGun store was moved to the data directory {} (TOPGUN_DATA_DIR).\n\
         See CHANGELOG.md, \"Changed (on-disk format)\". This file stays here so that a server binary older than this\n\
         layout refuses to start instead of serving an empty store. Do not delete it while such a binary may be started\n\
         with this path.\n",
        data_dir.to_string_lossy()
    )
    .into_bytes()
}

/// A temporary root with the three paths a server start works on.
#[allow(clippy::struct_field_names)]
struct Root {
    _dir: TempDir,
    root: PathBuf,
    /// `D`.
    d: PathBuf,
    /// `L`.
    l: PathBuf,
    /// `K`.
    k: PathBuf,
}

impl Root {
    fn new() -> Self {
        let dir = tempdir().expect("tempdir");
        let root = dir.path().to_path_buf();
        Self {
            d: root.join("topgun-data"),
            l: root.join("topgun.redb"),
            k: root.join("topgun.redb.pre-migration"),
            root,
            _dir: dir,
        }
    }

    fn location(&self) -> StoreLocation {
        StoreLocation {
            data_dir: self.d.clone(),
            data_dir_source: DataDirSource::DerivedFromRedbPath,
            legacy_path: Some(self.l.clone()),
        }
    }

    fn engine_path(&self) -> PathBuf {
        self.d.join("engine.redb")
    }

    fn stub_tmp(&self) -> PathBuf {
        PathBuf::from(format!("{}.stub-tmp", self.l.display()))
    }
}

fn exists(path: &Path) -> bool {
    fs::symlink_metadata(path).is_ok()
}

// ---------------------------------------------------------------------------
// Legacy fixtures
// ---------------------------------------------------------------------------

/// One table of a legacy fixture.
#[derive(Clone)]
struct LegacyTableSpec {
    map: String,
    backup: bool,
    rows: Vec<(String, Vec<u8>)>,
}

fn table(map: &str, backup: bool, rows: Vec<(String, Vec<u8>)>) -> LegacyTableSpec {
    LegacyTableSpec {
        map: map.to_string(),
        backup,
        rows,
    }
}

fn lww_bytes(tag: &str) -> Vec<u8> {
    rmp_serde::to_vec_named(&RecordValue::Lww {
        value: Value::String(tag.to_string()),
        timestamp: Timestamp {
            millis: 1_000,
            counter: 1,
            node_id: "n".to_string(),
        },
    })
    .expect("encode")
}

fn or_bytes(tag: &str) -> Vec<u8> {
    let ts = |millis| Timestamp {
        millis,
        counter: 3,
        node_id: "node-a".to_string(),
    };
    rmp_serde::to_vec_named(&RecordValue::OrMap {
        records: vec![
            OrMapEntry {
                value: Value::String(tag.to_string()),
                tag: format!("{tag}-1"),
                timestamp: ts(10),
            },
            OrMapEntry {
                value: Value::Int(42),
                tag: format!("{tag}-2"),
                timestamp: ts(11),
            },
        ],
        tombstones: vec![format!("{tag}-0"), format!("{tag}-9")],
    })
    .expect("encode")
}

fn rows(prefix: &str, n: usize) -> Vec<(String, Vec<u8>)> {
    (0..n)
        .map(|i| {
            let key = format!("{prefix}{i:05}");
            let value = lww_bytes(&format!("{prefix}-{i}"));
            (key, value)
        })
        .collect()
}

/// Writes `tables` into a new redb file at `path`, each under the table name
/// `table_name_for` gives it (an empty table is created with no row).
fn write_legacy(path: &Path, tables: &[LegacyTableSpec]) {
    let db = redb::Database::create(path).expect("legacy create");
    let txn = db.begin_write().expect("begin_write");
    for spec in tables {
        let name = table_name_for(&spec.map, spec.backup).expect("storable name");
        let mut t = txn.open_table(table_def(&name)).expect("open legacy table");
        for (key, value) in &spec.rows {
            t.insert(key.as_str(), value.as_slice()).expect("insert");
        }
    }
    txn.commit().expect("commit");
}

/// A fixture holding every kind of table the migration must carry.
fn rich_fixture() -> Vec<LegacyTableSpec> {
    let long_raw = format!("-{}", "a".repeat(599));
    let long_identifier = "b".repeat(600);
    vec![
        table("users", false, rows("u", 3)),
        table("users", true, rows("u", 2)),
        table("_internal", false, rows("i", 2)),
        table("_topgun_tombstone_cursors_v2", false, rows("c", 1)),
        table("_topgun_device_credentials", false, rows("d", 1)),
        table("__topgun_policies", false, rows("p", 1)),
        table("user-profiles", false, rows("up", 4)),
        table("user-profiles", true, rows("up", 2)),
        table("users/profiles", false, rows("us", 2)),
        table("map__a-b", false, rows("ma", 1)),
        table("mapr__a-b", false, rows("mr", 1)),
        table("maprb__a-b", false, rows("mb", 1)),
        table(&long_raw, false, rows("lr", 2)),
        table(&long_identifier, false, rows("li", 2)),
        table("empty_primary", false, Vec::new()),
        table("empty_backup", true, Vec::new()),
        table(
            "or_map",
            false,
            vec![
                ("o1".to_string(), or_bytes("o1")),
                ("o2".to_string(), or_bytes("o2")),
            ],
        ),
    ]
}

/// A small fixture: three maps with 25 rows each.
fn small_fixture() -> Vec<LegacyTableSpec> {
    vec![
        table("alpha", false, rows("a", 25)),
        table("beta-1", false, rows("b", 25)),
        table("gamma", true, rows("g", 25)),
    ]
}

/// The legacy `list_maps`: every primary table's map name, sorted.
fn legacy_list_maps(tables: &[LegacyTableSpec]) -> Vec<String> {
    let mut names: Vec<String> = tables
        .iter()
        .filter(|t| !t.backup)
        .map(|t| t.map.clone())
        .collect();
    names.sort();
    names.dedup();
    names
}

/// The ascending legacy table names of `tables`.
fn legacy_table_names(tables: &[LegacyTableSpec]) -> Vec<String> {
    let mut names: Vec<String> = tables
        .iter()
        .map(|t| table_name_for(&t.map, t.backup).expect("storable"))
        .collect();
    names.sort();
    names
}

// ---------------------------------------------------------------------------
// Observing the engine and the file system
// ---------------------------------------------------------------------------

type Dump = Vec<(Vec<u8>, Vec<u8>)>;

/// Every key and value of the engine's key space.
fn dump_engine<E: KvEngine>(engine: &E) -> Dump {
    let snapshot = engine.snapshot().expect("snapshot");
    let entries = snapshot
        .iter(KeyRange::default())
        .expect("iter")
        .map(|entry| entry.expect("entry"))
        .collect();
    entries
}

/// The dump of the engine file at `path`, through an engine opened for it and
/// dropped before this returns.
fn dump_engine_file(path: &Path) -> Dump {
    let engine = RedbKvEngine::open(path).expect("engine open for the dump");
    let dump = dump_engine(&engine);
    drop(engine);
    dump
}

/// The migration mark of the engine file at `path`.
fn mark_of_file(path: &Path) -> Option<Vec<u8>> {
    let engine = RedbKvEngine::open(path).expect("engine open for the mark");
    let mark = EngineMapDataStore::<RedbKvEngine>::read_migration_mark(&engine).expect("mark");
    drop(engine);
    mark
}

/// Every path under a root, and the bytes of every regular file outside the
/// data directory.
#[derive(Debug, PartialEq, Eq)]
struct Tree {
    paths: Vec<PathBuf>,
    files: BTreeMap<PathBuf, Vec<u8>>,
}

fn tree(root: &Path, data_dir: &Path) -> Tree {
    fn walk(path: &Path, data_dir: &Path, out: &mut Tree) {
        out.paths.push(path.to_path_buf());
        let Ok(meta) = fs::symlink_metadata(path) else {
            return;
        };
        if meta.is_dir() {
            let mut entries: Vec<PathBuf> = fs::read_dir(path)
                .expect("read_dir")
                .map(|e| e.expect("dir entry").path())
                .collect();
            entries.sort();
            for entry in entries {
                walk(&entry, data_dir, out);
            }
        } else if meta.is_file() && !path.starts_with(data_dir) {
            out.files
                .insert(path.to_path_buf(), fs::read(path).expect("read file"));
        }
    }
    let mut out = Tree {
        paths: Vec::new(),
        files: BTreeMap::new(),
    };
    walk(root, data_dir, &mut out);
    out.paths.sort();
    out
}

/// Asserts `l` is the stub naming `d`, exactly as R1 writes it.
fn assert_stub(l: &Path, d: &Path) {
    let meta = fs::symlink_metadata(l).expect("the stub exists");
    assert!(meta.is_file(), "the stub is a regular file");
    assert!(meta.len() <= STUB_LIMIT, "the stub is {} bytes", meta.len());
    let bytes = fs::read(l).expect("read stub");
    assert_eq!(&bytes[..MARKER_LINE.len()], MARKER_LINE, "marker line");
    assert_eq!(
        String::from_utf8_lossy(&bytes),
        String::from_utf8_lossy(&stub_text(d)),
        "the stub's text"
    );
}

// ---------------------------------------------------------------------------
// Log capture
// ---------------------------------------------------------------------------

/// One captured `tracing` event.
#[derive(Debug, Clone)]
struct Event {
    level: tracing::Level,
    message: String,
    fields: BTreeMap<String, String>,
}

impl Event {
    /// The message and every field, as one line.
    fn text(&self) -> String {
        use std::fmt::Write as _;
        let mut line = self.message.clone();
        for (name, value) in &self.fields {
            let _ = write!(line, " {name}={value}");
        }
        line
    }

    /// A field's value, without the quotes a `Debug` rendering adds.
    fn field(&self, name: &str) -> Option<String> {
        self.fields.get(name).map(|v| unquote(v))
    }

    fn is_store_line(&self) -> bool {
        self.message.starts_with("store ")
    }
}

fn unquote(value: &str) -> String {
    if value.len() >= 2 && value.starts_with('"') && value.ends_with('"') {
        value[1..value.len() - 1].to_string()
    } else {
        value.to_string()
    }
}

#[derive(Default)]
struct FieldVisitor {
    message: String,
    fields: BTreeMap<String, String>,
}

impl tracing::field::Visit for FieldVisitor {
    fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
        if field.name() == "message" {
            self.message = value.to_string();
        } else {
            self.fields
                .insert(field.name().to_string(), value.to_string());
        }
    }

    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        if field.name() == "message" {
            self.message = format!("{value:?}");
        } else {
            self.fields
                .insert(field.name().to_string(), format!("{value:?}"));
        }
    }
}

struct EventCapture(Arc<Mutex<Vec<Event>>>);

impl<S: tracing::Subscriber> tracing_subscriber::Layer<S> for EventCapture {
    fn on_event(
        &self,
        event: &tracing::Event<'_>,
        _ctx: tracing_subscriber::layer::Context<'_, S>,
    ) {
        let mut visitor = FieldVisitor::default();
        event.record(&mut visitor);
        self.0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(Event {
                level: *event.metadata().level(),
                message: visitor.message,
                fields: visitor.fields,
            });
    }
}

/// Runs `body` under a thread-local capture and returns what it logged.
///
/// Captures are serialised: installing or dropping a subscriber rebuilds the
/// process-wide callsite-interest cache, and a capture running beside that on
/// another thread could lose an event.
fn captured<T>(body: impl FnOnce() -> T) -> (T, Vec<Event>) {
    use tracing_subscriber::layer::SubscriberExt as _;
    static SERIAL: Mutex<()> = Mutex::new(());
    let _serial = SERIAL
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let sink: Arc<Mutex<Vec<Event>>> = Arc::new(Mutex::new(Vec::new()));
    let subscriber = tracing_subscriber::registry().with(EventCapture(Arc::clone(&sink)));
    let guard = tracing::subscriber::set_default(subscriber);
    let out = body();
    drop(guard);
    let events = sink
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone();
    (out, events)
}

/// `open_location_with`, logged.
fn open_logged(
    location: &StoreLocation,
    io: &MigrationIo,
) -> (anyhow::Result<RedbDataStore>, Vec<Event>) {
    captured(|| RedbDataStore::open_location_with(location, io))
}

fn error_text(err: &anyhow::Error) -> String {
    format!("{err:#}")
}

/// Asserts a refusal: an error carrying `line`, and exactly one ERROR event
/// carrying it. Returns the error's text.
fn assert_refused(result: anyhow::Result<RedbDataStore>, events: &[Event], line: &str) -> String {
    let Err(err) = result else {
        panic!("expected the refusal {line:?}, but the store opened");
    };
    let text = error_text(&err);
    assert!(
        text.contains(line),
        "the error {text:?} does not carry {line:?}"
    );
    let logged = events
        .iter()
        .filter(|e| e.level == tracing::Level::ERROR && e.text().contains(line))
        .count();
    assert_eq!(
        logged, 1,
        "exactly one ERROR line carries {line:?}; events: {events:#?}"
    );
    text
}

fn count_lines(events: &[Event], level: tracing::Level, needle: &str) -> usize {
    events
        .iter()
        .filter(|e| e.level == level && e.text().contains(needle))
        .count()
}

/// A data directory holding a store with one record, closed cleanly.
fn make_store_dir(d: &Path) {
    fs::create_dir_all(d).expect("create data dir");
    let engine = Arc::new(RedbKvEngine::open(&d.join("engine.redb")).expect("engine open"));
    let store = EngineMapDataStore::open(Arc::clone(&engine)).expect("store open");
    let bytes = lww_bytes("kept");
    store
        .write_records(
            &[RecordOp::Put {
                map: "kept",
                backup: false,
                key: "k",
                bytes: &bytes,
            }],
            WriteOptions { sync: true },
        )
        .expect("write a record");
    drop(store);
    drop(engine);
}

/// Every `(key, value bytes)` the store holds for `(map, backup)`, by key.
fn records_of(store: &RedbDataStore, map: &str, backup: bool) -> Vec<(String, Vec<u8>)> {
    let mut out = Vec::new();
    store
        .visit_records(map, backup, &mut |k, v| {
            out.push((k.to_string(), v.to_vec()));
            Ok(())
        })
        .expect("visit records");
    out.sort();
    out
}

/// Asserts the store holds exactly the rows of every fixture table.
fn assert_holds_fixture(store: &RedbDataStore, tables: &[LegacyTableSpec]) {
    for spec in tables {
        let mut expected = spec.rows.clone();
        expected.sort();
        assert_eq!(
            records_of(store, &spec.map, spec.backup),
            expected,
            "table of map {:?} (backup {})",
            spec.map.chars().take(40).collect::<String>(),
            spec.backup
        );
    }
}

/// The first engine error in an error's chain.
fn first_kv_error(err: &anyhow::Error) -> Option<&KvError> {
    err.downcast_ref::<KvError>()
        .or_else(|| err.chain().find_map(|e| e.downcast_ref::<KvError>()))
}

/// Whether an engine error says the file is held by another open.
fn kv_says_already_open(err: &KvError) -> bool {
    match err {
        KvError::Other(inner) => matches!(
            inner.downcast_ref::<redb::DatabaseError>(),
            Some(redb::DatabaseError::DatabaseAlreadyOpen)
        ),
        _ => false,
    }
}

// ---------------------------------------------------------------------------
// m01–m22
// ---------------------------------------------------------------------------

#[test]
fn m01_a_fresh_start_creates_the_data_directory_and_the_stub() {
    use std::os::unix::fs::MetadataExt as _;
    let r = Root::new();
    let location = r.location();

    let (opened, events) = open_logged(&location, &MigrationIo::default());
    let store = opened.expect("a fresh start opens");
    drop(store);

    let mut format_rows = vec![
        (meta_key(META_FORMAT), FORMAT_VERSION.to_be_bytes().to_vec()),
        (meta_key(META_NEXT_MAP_ID), 1u32.to_be_bytes().to_vec()),
        (meta_key(META_ACK_MODE), vec![0u8]),
    ];
    format_rows.sort();
    assert_eq!(
        dump_engine_file(&r.engine_path()),
        format_rows,
        "a fresh store holds exactly the format rows"
    );
    assert_stub(&r.l, &r.d);
    assert!(!exists(&r.stub_tmp()), "no temporary stub is left");
    assert!(
        !events.iter().any(|e| e.text().contains("migrat")),
        "a fresh start migrates nothing: {events:#?}"
    );

    // A restart leaves the stub as it is: same bytes, same file.
    let before = fs::read(&r.l).expect("read stub");
    let inode = fs::metadata(&r.l).expect("stub metadata").ino();
    let store = RedbDataStore::open_location(&location).expect("the restart opens");
    drop(store);
    assert_eq!(fs::read(&r.l).expect("read stub"), before, "no rewrite");
    assert_eq!(
        fs::metadata(&r.l).expect("stub metadata").ino(),
        inode,
        "the stub is not replaced"
    );

    // An existing store whose old path was deleted gets the stub back.
    fs::remove_file(&r.l).expect("delete the stub");
    let store = RedbDataStore::open_location(&location).expect("opens without a stub");
    drop(store);
    assert_stub(&r.l, &r.d);

    // `new(path)` writes no stub anywhere.
    let other = tempdir().expect("tempdir");
    let data_dir = other.path().join("store-dir");
    let store = RedbDataStore::new(&data_dir).expect("new opens a data directory");
    drop(store);
    let outside: Vec<PathBuf> = tree(other.path(), &data_dir)
        .paths
        .into_iter()
        .filter(|p| p != other.path() && !p.starts_with(&data_dir))
        .collect();
    assert!(outside.is_empty(), "new(path) created {outside:?}");
    assert!(data_dir.join("engine.redb").is_file());
}

#[test]
fn m02_a_legacy_file_is_migrated_byte_for_byte_and_kept_under_the_pre_migration_name() {
    let r = Root::new();
    let fixture = rich_fixture();
    write_legacy(&r.l, &fixture);
    let legacy_bytes = fs::read(&r.l).expect("read fixture");

    let store = RedbDataStore::open_location(&r.location()).expect("the legacy file migrates");
    assert_holds_fixture(&store, &fixture);
    drop(store);

    assert_eq!(
        fs::read(&r.k).expect("the kept file"),
        legacy_bytes,
        "the legacy file is kept byte for byte"
    );
    assert_stub(&r.l, &r.d);
    assert_eq!(mark_of_file(&r.engine_path()), None, "no mark is left");
}

#[tokio::test]
async fn m03_list_maps_after_a_migration_equals_the_legacy_list_maps() {
    let r = Root::new();
    let fixture = rich_fixture();
    write_legacy(&r.l, &fixture);

    let store = RedbDataStore::open_location(&r.location()).expect("the legacy file migrates");
    assert_eq!(
        store.list_maps().await.expect("list_maps"),
        legacy_list_maps(&fixture)
    );
}

/// The fixture of m04: small enough for 120 crash cases, with a table split
/// over many batches, an empty table and a backup table.
fn m04_fixture() -> Vec<LegacyTableSpec> {
    vec![
        table("m04_a", false, rows("a", 30)),
        table("m04_b-raw", false, rows("b", 30)),
        table("m04_c", true, rows("c", 12)),
        table("m04_empty", false, Vec::new()),
        table("m04_d", false, rows("d", 20)),
    ]
}

fn m04_io() -> MigrationIo {
    MigrationIo {
        batch_rows: 7,
        durable_every: Duration::ZERO,
        ..MigrationIo::default()
    }
}

/// Runs a migration of a copy of `template` with `run_migration` over a
/// [`CrashableBackend`], killing the backend at each of `deaths` (counted
/// backend ops after the run's start) and crashing it in `mode` before the
/// next run resumes from its mark. Returns the final dump, the number of
/// deaths that happened and the ops of the first run.
fn m04_migrate(
    template: &Path,
    tables: &[LegacyTableSpec],
    deaths: &[u64],
    mode: CrashMode,
    seed: u64,
) -> (Dump, usize, u64) {
    let dir = tempdir().expect("tempdir");
    let l = dir.path().join("topgun.redb");
    fs::copy(template, &l).expect("copy fixture");
    let paths = MigrationPaths {
        data_dir: dir.path().join("topgun-data"),
        legacy_path: l.clone(),
        kept_path: dir.path().join("topgun.redb.pre-migration"),
    };
    let io = m04_io();
    let initial = MigrationMark {
        phase: MigrationPhase::Copying,
        source: l.to_str().expect("UTF-8 path").to_string(),
        table: legacy_table_names(tables)[0].clone(),
        key: None,
    }
    .encode();

    let mut backend = CrashableBackend::new();
    let mut engine = Arc::new(RedbKvEngine::with_backend(backend.clone()).expect("engine"));
    let mut first_run = true;
    let mut deaths_done = 0usize;
    let mut first_run_ops = 0u64;
    loop {
        if !first_run
            && EngineMapDataStore::<RedbKvEngine>::read_migration_mark(&engine)
                .expect("read mark")
                .is_none()
        {
            break;
        }
        let (store, mark_bytes) =
            EngineMapDataStore::open_for_migration(Arc::clone(&engine), &initial)
                .expect("open for migration");
        first_run = false;
        let mark = MigrationMark::decode(&mark_bytes).expect("decode mark");
        let legacy = if is_legacy_file(&l).expect("legacy check") {
            let db = redb::Database::open(&l).expect("legacy open");
            let tables = read_legacy_catalog(&db).expect("catalog");
            Some(LegacySource { db, tables })
        } else {
            None
        };
        let start = backend.ops_so_far();
        let die = deaths.get(deaths_done).map(|n| start + n);
        backend.set_die_at(die);
        let outcome = run_migration(&engine, &store, legacy, &mark, &paths, &io);
        let died = die.is_some_and(|at| backend.ops_so_far() > at);
        if deaths_done == 0 && first_run_ops == 0 {
            first_run_ops = backend.ops_so_far() - start;
        }
        match outcome {
            Ok(()) => {
                assert!(!died, "the run returned Ok after its backend died");
                backend.set_die_at(None);
                drop(store);
                break;
            }
            Err(e) => {
                assert!(died, "the run failed without a death: {e:#}");
                // The crash in the redb order: the image first, then the dead
                // engine is dropped, never before (its destructor must not
                // reach the image).
                let image = backend.crash_image(mode, seed.wrapping_add(deaths_done as u64));
                assert_eq!(backend.retained_bytes(), 0, "the dead backend keeps bytes");
                drop(store);
                assert_eq!(Arc::strong_count(&engine), 1, "the dead engine is shared");
                drop(engine);
                backend = image;
                engine = Arc::new(
                    RedbKvEngine::with_backend(backend.clone()).expect("reopen after the crash"),
                );
                deaths_done += 1;
            }
        }
    }
    (dump_engine(&*engine), deaths_done, first_run_ops)
}

#[test]
fn m04_k_interrupted_migrations_end_byte_for_byte_equal_to_an_uninterrupted_one() {
    let template_dir = tempdir().expect("tempdir");
    let template = template_dir.path().join("fixture.redb");
    let tables = m04_fixture();
    write_legacy(&template, &tables);

    let (reference, none, run_ops) =
        m04_migrate(&template, &tables, &[], CrashMode::DropUnsynced, 0);
    assert_eq!(none, 0);
    assert!(run_ops > 0, "the uninterrupted run made backend ops");
    assert!(
        reference
            .iter()
            .all(|(k, _)| k != &meta_key(META_MIGRATION)),
        "the uninterrupted migration leaves no mark"
    );

    for k in 1..=3u64 {
        // Each death lands within the first 1/(2k) of a run's ops, so a
        // resumed run always has more work left than its death point: every
        // case dies exactly k times.
        let span = (run_ops / (2 * k)).max(1);
        for seed in 0..20u64 {
            for mode in [CrashMode::DropUnsynced, CrashMode::KeepRandomWrites] {
                let mut rng = StdRng::seed_from_u64(seed * 31 + k);
                let deaths: Vec<u64> = (0..k).map(|_| rng.random_range(1..=span)).collect();
                let (dump, died, _) = m04_migrate(&template, &tables, &deaths, mode, seed);
                assert_eq!(
                    died as u64, k,
                    "k={k} seed={seed} mode={mode:?} deaths={deaths:?}: deaths that happened"
                );
                assert!(
                    dump == reference,
                    "k={k} seed={seed} mode={mode:?} deaths={deaths:?}: the final engine \
                     content differs from the uninterrupted migration's"
                );
            }
        }
    }
}

/// The environment variable that makes m05 run as a child.
const M05_CHILD_ENV: &str = "TOPGUN_TEST_M05_CHILD";
const M05_NAME: &str =
    "storage::datastores::redb::migration_tests::m05_a_killed_process_resumes_from_its_mark";
const M05_RESULT: &str = "M05-CHILD-RESULT ";
const M05_TABLES: usize = 12;
const M05_ROWS: usize = 3_000;

fn m05_fixture() -> Vec<LegacyTableSpec> {
    (0..M05_TABLES)
        .map(|t| table(&format!("t{t:02}"), false, rows("k", M05_ROWS)))
        .collect()
}

fn m05_io(abort_at: Option<AbortAt>) -> MigrationIo {
    MigrationIo {
        batch_rows: 1_000,
        durable_every: Duration::ZERO,
        progress_every: Duration::ZERO,
        abort_at,
        ..MigrationIo::default()
    }
}

fn m05_abort(spec: &str) -> Option<AbortAt> {
    match spec.split_once(':') {
        Some(("batch", n)) => Some(AbortAt::AfterBatch(n.parse().expect("batch number"))),
        Some(("step", n)) => Some(AbortAt::AfterStep(match n {
            "5" => MigrationStep::MarkCopied,
            "7" => MigrationStep::MarkVerified,
            "9" => MigrationStep::KeepLegacy,
            "10" => MigrationStep::WriteMarker,
            other => panic!("no abort step {other}"),
        })),
        None if spec == "marker" => Some(AbortAt::MarkerBeforeRename),
        None if spec == "stub-create" => Some(AbortAt::StubAfterCreate),
        None if spec == "none" => None,
        _ => panic!("unknown abort spec {spec:?}"),
    }
}

/// The child half of m05: one server start on the root it is given, logging
/// to stdout, then a result line (unless it aborted first).
fn m05_child(spec: &str) {
    let (root, abort) = spec.split_once('\n').expect("child spec");
    let root = PathBuf::from(root);
    let location = StoreLocation {
        data_dir: root.join("topgun-data"),
        data_dir_source: DataDirSource::DerivedFromRedbPath,
        legacy_path: Some(root.join("topgun.redb")),
    };
    let subscriber = tracing_subscriber::fmt()
        .with_writer(std::io::stdout)
        .with_ansi(false)
        .finish();
    let _guard = tracing::subscriber::set_default(subscriber);
    let result = RedbDataStore::open_location_with(&location, &m05_io(m05_abort(abort)));
    match result {
        Ok(store) => {
            drop(store);
            println!("{M05_RESULT}ok");
        }
        Err(e) => println!("{M05_RESULT}err {e:#}"),
    }
}

struct ChildRun {
    success: bool,
    stdout: String,
}

/// Runs one child start with a hard deadline; a child still running at the
/// deadline is killed and fails the test as a hang.
fn m05_run_child(root: &Path, abort: &str, logs: &Path, label: &str) -> ChildRun {
    let out_path = logs.join(format!("{label}.out"));
    let err_path = logs.join(format!("{label}.err"));
    let mut child = std::process::Command::new(std::env::current_exe().expect("test binary"))
        .args([M05_NAME, "--exact", "--nocapture", "--test-threads", "1"])
        .env_remove("DATABASE_URL")
        .env(M05_CHILD_ENV, format!("{}\n{abort}", root.display()))
        .stdout(fs::File::create(&out_path).expect("stdout file"))
        .stderr(fs::File::create(&err_path).expect("stderr file"))
        .spawn()
        .expect("the child starts");
    let deadline = Instant::now() + CHILD_DEADLINE;
    let status = loop {
        if let Some(status) = child.try_wait().expect("try_wait") {
            break status;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            panic!("m05 child {label} ({abort}) still running after {CHILD_DEADLINE:?}: a hang");
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    ChildRun {
        success: status.success(),
        stdout: fs::read_to_string(&out_path).unwrap_or_default(),
    }
}

/// The largest `rows_copied_this_run` a run's progress lines report.
fn rows_copied_this_run(stdout: &str) -> Option<u64> {
    stdout
        .lines()
        .filter(|line| line.contains("store migration progress"))
        .filter_map(|line| {
            let rest = line.split("rows_copied_this_run=").nth(1)?;
            rest.split(|c: char| !c.is_ascii_digit())
                .next()?
                .parse()
                .ok()
        })
        .max()
}

#[test]
fn m05_a_killed_process_resumes_from_its_mark() {
    if let Ok(spec) = std::env::var(M05_CHILD_ENV) {
        m05_child(&spec);
        return;
    }

    let template_dir = tempdir().expect("tempdir");
    let template = template_dir.path().join("fixture.redb");
    write_legacy(&template, &m05_fixture());
    let template_bytes = fs::read(&template).expect("read fixture");
    let total_rows = (M05_TABLES * M05_ROWS) as u64;

    let reference = {
        let r = Root::new();
        fs::copy(&template, &r.l).expect("copy fixture");
        let store = RedbDataStore::open_location_with(&r.location(), &m05_io(None))
            .expect("the uninterrupted migration");
        drop(store);
        dump_engine_file(&r.engine_path())
    };

    let sequences: [(&str, &[&str]); 7] = [
        ("batches", &["batch:1", "batch:3", "batch:7"]),
        ("step5", &["step:5"]),
        ("step7", &["step:7"]),
        ("step9", &["step:9"]),
        ("step10", &["step:10"]),
        ("marker", &["marker"]),
        ("stub-create", &["stub-create"]),
    ];
    let logs = tempdir().expect("log dir");
    for (name, aborts) in sequences {
        let r = Root::new();
        fs::copy(&template, &r.l).expect("copy fixture");
        let mut runs: Vec<ChildRun> = Vec::new();
        for (i, abort) in aborts.iter().enumerate() {
            let run = m05_run_child(&r.root, abort, logs.path(), &format!("{name}-{i}"));
            assert!(
                !run.success && !run.stdout.contains(M05_RESULT),
                "{name}: the run with abort {abort} did not abort; stdout:\n{}",
                run.stdout
            );
            if exists(&r.l) {
                let bytes = fs::read(&r.l).expect("read L");
                assert!(
                    is_legacy_file(&r.l).expect("legacy check") || bytes == stub_text(&r.d),
                    "{name}: after the abort {abort} the old path holds {} bytes that are \
                     neither the legacy file nor the whole stub",
                    bytes.len()
                );
            }
            runs.push(run);
        }
        let last = m05_run_child(&r.root, "none", logs.path(), &format!("{name}-final"));
        assert!(
            last.success && last.stdout.contains(&format!("{M05_RESULT}ok")),
            "{name}: the final run did not complete; stdout:\n{}",
            last.stdout
        );
        runs.push(last);

        if name == "batches" {
            for (i, run) in runs.iter().enumerate().skip(1) {
                assert!(
                    run.stdout.contains("resuming from table"),
                    "{name}: run {i} resumed the copy without logging it"
                );
                let copied = rows_copied_this_run(&run.stdout)
                    .unwrap_or_else(|| panic!("{name}: run {i} logged no progress line"));
                assert!(
                    copied < total_rows,
                    "{name}: run {i} copied {copied} rows, not fewer than the {total_rows} of \
                     the fixture: it did not resume from its mark"
                );
            }
        }

        assert!(
            dump_engine_file(&r.engine_path()) == reference,
            "{name}: the final engine content differs from the uninterrupted migration's"
        );
        assert_eq!(
            fs::read(&r.k).expect("the kept file"),
            template_bytes,
            "{name}: the kept file"
        );
        assert_stub(&r.l, &r.d);
    }
}

#[test]
fn m06_a_migration_whose_copy_differs_is_refused_and_serves_nothing() {
    let r = Root::new();
    write_legacy(&r.l, &small_fixture());
    let legacy_bytes = fs::read(&r.l).expect("read fixture");
    let location = r.location();
    let unfinished = format!(
        "{} is left unfinished and is refused until it is removed",
        r.d.display()
    );

    let corrupt = MigrationIo {
        corrupt_one_copied_byte: true,
        ..MigrationIo::default()
    };
    let (first, first_events) = open_logged(&location, &corrupt);
    let Err(first) = first else {
        panic!("a migration whose copy differs must not open");
    };
    let first_text = error_text(&first);
    assert!(
        first_text.contains("store migration: verification failed for ")
            && first_text.contains(&unfinished),
        "{first_text}"
    );
    assert_eq!(
        fs::read(&r.l).expect("read L"),
        legacy_bytes,
        "L is untouched"
    );
    assert!(
        !exists(&r.k),
        "nothing is kept: the migration did not finish"
    );

    let (second, second_events) = open_logged(&location, &MigrationIo::default());
    let Err(second) = second else {
        panic!("the next start must refuse the unfinished store too");
    };
    assert!(
        error_text(&second).contains("store migration: verification failed for "),
        "{second:#}"
    );
    let lines = |events: &[Event]| -> Vec<String> {
        events
            .iter()
            .filter(|e| {
                e.level == tracing::Level::ERROR && e.text().contains("verification failed")
            })
            .map(Event::text)
            .collect()
    };
    let first_lines = lines(&first_events);
    assert_eq!(first_lines.len(), 1, "{first_events:#?}");
    assert_eq!(lines(&second_events), first_lines, "the same line again");
    assert_eq!(
        fs::read(&r.l).expect("read L"),
        legacy_bytes,
        "L is untouched"
    );
    assert!(
        mark_of_file(&r.engine_path()).is_some(),
        "the store keeps its mark, so it is never served"
    );
}

#[test]
fn m07_a_legacy_table_that_is_not_a_store_table_refuses_the_migration() {
    type Build = fn(&redb::WriteTransaction);
    let cases: [(&str, Build); 5] = [
        ("other_table", |txn| {
            let def: redb::TableDefinition<&str, &[u8]> = redb::TableDefinition::new("other_table");
            txn.open_table(def)
                .unwrap()
                .insert("k", b"v".as_slice())
                .unwrap();
        }),
        ("map__a-b", |txn| {
            let def: redb::TableDefinition<&str, &[u8]> = redb::TableDefinition::new("map__a-b");
            txn.open_table(def)
                .unwrap()
                .insert("k", b"v".as_slice())
                .unwrap();
        }),
        ("map__users", |txn| {
            let def: redb::TableDefinition<u64, u64> = redb::TableDefinition::new("map__users");
            txn.open_table(def).unwrap().insert(1, 1).unwrap();
        }),
        ("map__multi", |txn| {
            let def: redb::MultimapTableDefinition<&str, &[u8]> =
                redb::MultimapTableDefinition::new("map__multi");
            txn.open_multimap_table(def)
                .unwrap()
                .insert("k", b"v".as_slice())
                .unwrap();
        }),
        ("mapr__users", |txn| {
            let def: redb::TableDefinition<&str, &[u8]> = redb::TableDefinition::new("mapr__users");
            txn.open_table(def)
                .unwrap()
                .insert("k", lww_bytes("v").as_slice())
                .unwrap();
        }),
    ];
    for (name, build) in cases {
        let r = Root::new();
        {
            let db = redb::Database::create(&r.l).expect("create");
            let txn = db.begin_write().expect("begin_write");
            // A good table beside the offending one: the refusal is not about
            // an empty file.
            txn.open_table(table_def("map__users_ok"))
                .unwrap()
                .insert("k", lww_bytes("v").as_slice())
                .unwrap();
            build(&txn);
            txn.commit().expect("commit");
        }
        let legacy_bytes = fs::read(&r.l).expect("read fixture");

        let (opened, events) = open_logged(&r.location(), &MigrationIo::default());
        assert!(opened.is_err(), "{name}: the migration must be refused");
        assert!(!exists(&r.d), "{name}: the data directory was created");
        assert!(
            events
                .iter()
                .any(|e| e.level == tracing::Level::ERROR && e.text().contains(name)),
            "{name}: no ERROR line names the table: {events:#?}"
        );
        assert_eq!(
            fs::read(&r.l).expect("read L"),
            legacy_bytes,
            "{name}: L is untouched"
        );
    }
}

#[test]
fn m08_every_refusing_row_of_the_location_table_touches_nothing() {
    // L3: the data directory holds something else.
    {
        let r = Root::new();
        fs::create_dir_all(&r.d).unwrap();
        fs::write(r.d.join("unrelated.txt"), b"not a store").unwrap();
        let before = tree(&r.root, &r.d);
        let (opened, events) = open_logged(&r.location(), &MigrationIo::default());
        assert_refused(
            opened,
            &events,
            &format!(
                "store location: {} is not a store directory, an empty directory or absent",
                r.d.display()
            ),
        );
        assert_eq!(tree(&r.root, &r.d), before, "L3 touched the tree");
    }

    // L7: a store with a mark this location cannot resume.
    {
        let r = Root::new();
        write_legacy(&r.l, &small_fixture());
        fs::create_dir_all(&r.d).unwrap();
        {
            let engine = Arc::new(RedbKvEngine::open(&r.engine_path()).expect("engine"));
            let foreign = MigrationMark {
                phase: MigrationPhase::Copying,
                source: "/elsewhere/topgun.redb".to_string(),
                table: "map__alpha".to_string(),
                key: None,
            }
            .encode();
            let (store, _) = EngineMapDataStore::open_for_migration(Arc::clone(&engine), &foreign)
                .expect("mark");
            drop(store);
            drop(engine);
        }
        let dump_before = dump_engine_file(&r.engine_path());
        let before = tree(&r.root, &r.d);
        let (opened, events) = open_logged(&r.location(), &MigrationIo::default());
        assert_refused(
            opened,
            &events,
            "store migration: refusing to resume the migration from /elsewhere/topgun.redb: ",
        );
        assert_eq!(tree(&r.root, &r.d), before, "L7 touched the tree");
        assert!(
            dump_engine_file(&r.engine_path()) == dump_before,
            "L7 changed the engine's content"
        );
    }

    // L9: a pre-migration file already exists.
    {
        let r = Root::new();
        write_legacy(&r.l, &small_fixture());
        fs::write(&r.k, b"an earlier kept file").unwrap();
        let before = tree(&r.root, &r.d);
        let (opened, events) = open_logged(&r.location(), &MigrationIo::default());
        assert_refused(
            opened,
            &events,
            &format!(
                "store migration: a pre-migration file already exists at {}; move it away before migrating {}",
                r.k.display(),
                r.l.display()
            ),
        );
        assert_eq!(tree(&r.root, &r.d), before, "L9 touched the tree");
    }

    // L10: the stub, and no store.
    {
        let r = Root::new();
        fs::write(&r.l, stub_text(&r.d)).unwrap();
        let before = tree(&r.root, &r.d);
        let (opened, events) = open_logged(&r.location(), &MigrationIo::default());
        assert_refused(
            opened,
            &events,
            &format!(
                "store location: {} says the store was moved to a data directory, but {} holds no store",
                r.l.display(),
                r.d.display()
            ),
        );
        assert_eq!(tree(&r.root, &r.d), before, "L10 touched the tree");
    }

    // L11: only the kept file.
    {
        let r = Root::new();
        write_legacy(&r.k, &small_fixture());
        let before = tree(&r.root, &r.d);
        let (opened, events) = open_logged(&r.location(), &MigrationIo::default());
        assert_refused(
            opened,
            &events,
            &format!(
                "store location: a pre-migration store file is kept at {}, but {} holds no store; \
                 restore the data directory, or rename {} to {} to migrate it again",
                r.k.display(),
                r.d.display(),
                r.k.display(),
                r.l.display()
            ),
        );
        assert_eq!(tree(&r.root, &r.d), before, "L11 touched the tree");
    }
}

#[test]
fn m09_the_same_place_rules() {
    let line = |d: &Path, l: &Path| {
        format!(
            "store location: TOPGUN_DATA_DIR ({}) and TOPGUN_REDB_PATH ({}) name the same place or one lies inside the other",
            d.display(),
            l.display()
        )
    };
    let dir = tempdir().expect("tempdir");
    let root = dir.path();
    let linked = root.join("d-linked");
    fs::create_dir(&linked).unwrap();
    std::os::unix::fs::symlink(&linked, root.join("link")).unwrap();
    let cases = [
        ("D == L", root.join("same"), root.join("same")),
        ("D inside L", root.join("x").join("data"), root.join("x")),
        (
            "L inside D",
            root.join("d"),
            root.join("d").join("topgun.redb"),
        ),
        ("L a symlink to D", linked.clone(), root.join("link")),
    ];
    for (name, d, l) in cases {
        let before = tree(root, &root.join("no-data-dir"));
        let location = StoreLocation {
            data_dir: d.clone(),
            data_dir_source: DataDirSource::TopgunDataDir,
            legacy_path: Some(l.clone()),
        };
        let (opened, events) = open_logged(&location, &MigrationIo::default());
        assert_refused(opened, &events, &line(&d, &l));
        assert_eq!(
            tree(root, &root.join("no-data-dir")),
            before,
            "{name}: something was created"
        );
    }
}

#[test]
fn m10_a_file_that_is_not_a_table_per_map_store_is_refused() {
    type Make = fn(&Path);
    let cases: [(&str, Make); 4] = [
        ("4 KiB of zeros", |l| fs::write(l, vec![0u8; 4096]).unwrap()),
        ("an empty file", |l| fs::write(l, b"").unwrap()),
        ("a directory", |l| fs::create_dir(l).unwrap()),
        ("the marker in a file of 16 385 bytes", |l| {
            let mut bytes = MARKER_LINE.to_vec();
            bytes.resize(16_385, b'x');
            fs::write(l, bytes).unwrap();
        }),
    ];
    for (name, make) in cases {
        let r = Root::new();
        make(&r.l);
        let before = tree(&r.root, &r.d);
        let (opened, events) = open_logged(&r.location(), &MigrationIo::default());
        assert_refused(
            opened,
            &events,
            &format!(
                "store migration: {} is not a table-per-map store file",
                r.l.display()
            ),
        );
        assert_eq!(tree(&r.root, &r.d), before, "{name}: the tree changed");
    }
}

#[test]
fn m11_a_store_with_a_legacy_file_beside_it_is_refused_and_touches_nothing() {
    type Make = fn(&Path);
    let legacy_files: [(&str, Make); 2] = [
        ("a table-per-map fixture", |l| {
            write_legacy(l, &small_fixture());
        }),
        ("an initialised redb file with no table", |l| {
            drop(redb::Database::create(l).expect("create"));
        }),
    ];
    for (name, make) in legacy_files {
        let r = Root::new();
        make_store_dir(&r.d);
        make(&r.l);
        let line = format!(
            "store location: two stores found: {} holds a store in the data-directory format and {} \
             is a table-per-map store file; the server serves only one. Move one of them aside: \
             rename {} to keep the data-directory store, or move {} away to migrate {} again",
            r.d.display(),
            r.l.display(),
            r.l.display(),
            r.d.display(),
            r.l.display()
        );
        let dump_before = dump_engine_file(&r.engine_path());
        let before = tree(&r.root, &r.d);

        let (opened, events) = open_logged(&r.location(), &MigrationIo::default());
        let first = assert_refused(opened, &events, &line);
        assert!(first.contains("two stores found"), "{name}");
        assert!(
            dump_engine_file(&r.engine_path()) == dump_before,
            "{name}: the engine's content changed"
        );
        assert_eq!(tree(&r.root, &r.d), before, "{name}: the tree changed");

        let (again, events) = open_logged(&r.location(), &MigrationIo::default());
        let second = assert_refused(again, &events, &line);
        assert_eq!(
            second, first,
            "{name}: the second start refuses the same way"
        );
        assert!(
            dump_engine_file(&r.engine_path()) == dump_before,
            "{name}: the engine's content changed on the second start"
        );
        assert_eq!(tree(&r.root, &r.d), before, "{name}: the tree changed");
    }
}

#[test]
fn m12_insufficient_free_space_refuses_and_touches_nothing() {
    let r = Root::new();
    write_legacy(&r.l, &small_fixture());
    let size = fs::metadata(&r.l).expect("fixture size").len();
    let required = (3 * size).div_ceil(2) + MIGRATION_SLACK;
    let legacy_bytes = fs::read(&r.l).expect("read fixture");
    let before = tree(&r.root, &r.d);

    let short = MigrationIo {
        available_space: Box::new(move |_| Ok(required - 1)),
        ..MigrationIo::default()
    };
    let (opened, events) = open_logged(&r.location(), &short);
    assert_refused(
        opened,
        &events,
        &format!(
            "store migration: refusing to start: {} bytes are free on the file system of {}, {} are \
             needed (1.5 \u{d7} the size of {}, plus 64 MiB); nothing was changed",
            required - 1,
            r.d.display(),
            required,
            r.l.display()
        ),
    );
    assert!(!exists(&r.d), "the data directory was created");
    assert_eq!(fs::read(&r.l).expect("read L"), legacy_bytes, "L changed");
    assert_eq!(tree(&r.root, &r.d), before, "the tree changed");

    let enough = MigrationIo {
        available_space: Box::new(move |_| Ok(required)),
        ..MigrationIo::default()
    };
    let store = RedbDataStore::open_location_with(&r.location(), &enough)
        .expect("exactly the required space migrates");
    drop(store);
    assert!(exists(&r.k), "the legacy file is kept");
    assert_stub(&r.l, &r.d);
}

#[test]
fn m13_the_kept_file_is_logged_at_every_start_while_it_exists() {
    let r = Root::new();
    write_legacy(&r.l, &small_fixture());
    let location = r.location();
    drop(RedbDataStore::open_location(&location).expect("the migration"));
    let line = format!(
        "store format: the pre-migration store file is kept at {}; delete it once this store is confirmed",
        r.k.display()
    );

    for start in 1..=2 {
        let (opened, events) = open_logged(&location, &MigrationIo::default());
        drop(opened.expect("the store opens"));
        assert_eq!(
            count_lines(&events, tracing::Level::INFO, &line),
            1,
            "start {start}: {events:#?}"
        );
    }

    fs::remove_file(&r.k).expect("delete the kept file");
    let (opened, events) = open_logged(&location, &MigrationIo::default());
    drop(opened.expect("the store opens"));
    assert!(
        !events
            .iter()
            .any(|e| e.text().contains("pre-migration store file is kept")),
        "{events:#?}"
    );
}

#[test]
fn m14_an_older_binary_cannot_open_the_stub_or_the_data_directory() {
    let r = Root::new();
    let store = RedbDataStore::open_location(&r.location()).expect("a fresh start");
    drop(store);
    let stub = fs::read(&r.l).expect("the stub the code wrote");
    let before = tree(&r.root, &r.root.join("no-data-dir"));

    match redb::Database::create(&r.l) {
        Ok(_) => panic!("redb opened the stub"),
        Err(e) => assert!(
            matches!(
                &e,
                redb::DatabaseError::Storage(redb::StorageError::Io(io))
                    if io.kind() == std::io::ErrorKind::InvalidData
            ),
            "the stub is refused as invalid data: {e:?}"
        ),
    }
    assert!(
        redb::Database::create(&r.d).is_err(),
        "redb opened the data directory"
    );
    assert_eq!(fs::read(&r.l).expect("read stub"), stub, "the stub changed");
    assert_eq!(
        fs::metadata(&r.l).expect("stub").len(),
        stub.len() as u64,
        "the stub's length changed"
    );
    assert_eq!(
        tree(&r.root, &r.root.join("no-data-dir")),
        before,
        "a path was created or removed"
    );

    // Control: an empty file is opened as a new database, which is why the
    // stub is never empty.
    let scratch = tempdir().expect("tempdir");
    let empty = scratch.path().join("empty.redb");
    fs::write(&empty, b"").unwrap();
    assert!(redb::Database::create(&empty).is_ok());
}

#[tokio::test]
#[allow(clippy::too_many_lines)]
async fn m15_wal_frames_left_by_an_older_binary_replay_into_the_migrated_store() {
    use crate::storage::wal::{
        Wal, WalEntry, WalFsyncPolicy, WalOp, WalRecovery, WalStorePayload, WalWriter,
    };

    const MAP: &str = "user-profiles";
    const PARTITION: u32 = 7;

    let stamped = |tag: &str, millis: u64| RecordValue::Lww {
        value: Value::String(tag.to_string()),
        timestamp: Timestamp {
            millis,
            counter: 0,
            node_id: "writer".to_string(),
        },
    };
    let store_frame = |key: &str, value: &RecordValue, sequence: u64| WalEntry {
        map: MAP.to_string(),
        key: key.to_string(),
        op: WalOp::Store {
            value: WalStorePayload::Record(value.clone()),
            expiration_time: None,
        },
        timestamp: match value {
            RecordValue::Lww { timestamp, .. } => Some(timestamp.clone()),
            _ => None,
        },
        sequence,
    };
    let k1_value = stamped("first", 100);
    let k2_value = stamped("second", 200);
    let frames = [
        store_frame("k1", &k1_value, 1),
        store_frame("k2", &k2_value, 2),
        WalEntry {
            map: MAP.to_string(),
            key: "k1".to_string(),
            op: WalOp::Remove,
            timestamp: None,
            sequence: 3,
        },
    ];

    let r = Root::new();
    let k0 = rmp_serde::to_vec_named(&stamped("zero", 50)).expect("encode");
    write_legacy(
        &r.l,
        &[table(MAP, false, vec![("k0".to_string(), k0.clone())])],
    );
    let wal_dir = r.root.join("wal");
    // The frames are written over a throwaway store; an older binary leaves
    // no `wal-collected`, and no `.applied` file while nothing is applied.
    {
        let scratch: Arc<dyn MapDataStore> = Arc::new(
            EngineMapDataStore::open(Arc::new(
                crate::storage::datastores::kv_engine::MemKvEngine::new(),
            ))
            .expect("store"),
        );
        let wal = WalWriter::open_over_store(
            wal_dir.clone(),
            WalFsyncPolicy::PerOp,
            Arc::clone(&scratch),
        )
        .expect("wal open");
        for frame in &frames {
            wal.append(PARTITION, frame).await.expect("wal append");
        }
        let released = Arc::downgrade(&wal);
        drop(wal);
        let deadline = Instant::now() + Duration::from_secs(30);
        while released.upgrade().is_some() {
            assert!(
                Instant::now() < deadline,
                "the WAL writer was never released"
            );
            tokio::time::sleep(Duration::from_millis(2)).await;
        }
        fs::remove_file(wal_dir.join(crate::storage::wal::WAL_COLLECTED_FILE)).unwrap();
    }

    let store = Arc::new(RedbDataStore::open_location(&r.location()).expect("the migration"));
    let inner: Arc<dyn MapDataStore> = store.clone();
    let wal = WalWriter::open_over_store(wal_dir, WalFsyncPolicy::PerOp, Arc::clone(&inner))
        .expect("wal reopen");
    WalRecovery::new(Arc::clone(&wal), Vec::new())
        .run(inner)
        .await
        .expect("recovery");

    let unreplayed = wal.unapplied(PARTITION).await.expect("unapplied");
    assert!(unreplayed.is_empty(), "{unreplayed:?}");
    assert_eq!(
        store
            .applied_watermarks()
            .expect("applied watermarks")
            .expect("a store that keeps applied watermarks")
            .by_partition[PARTITION as usize],
        3
    );
    assert_eq!(store.load(MAP, "k2").await.expect("load"), Some(k2_value));
    assert_eq!(store.load(MAP, "k1").await.expect("load"), None);
    assert_eq!(
        records_of(&store, MAP, false)
            .into_iter()
            .find(|(k, _)| k == "k0")
            .map(|(_, v)| v),
        Some(k0),
        "the migrated row is kept"
    );
}

#[test]
fn m16_progress_is_logged_at_least_once_per_interval() {
    const FIELDS: [&str; 5] = [
        "tables_done",
        "tables_total",
        "current_table",
        "rows_copied_this_run",
        "bytes_copied_this_run",
    ];
    // Three tables of 25 rows in batches of 10: three batches each.
    let r = Root::new();
    write_legacy(&r.l, &small_fixture());
    let every_batch = MigrationIo {
        batch_rows: 10,
        progress_every: Duration::ZERO,
        ..MigrationIo::default()
    };
    let (opened, events) = open_logged(&r.location(), &every_batch);
    drop(opened.expect("the migration"));
    let progress: Vec<&Event> = events
        .iter()
        .filter(|e| e.message.starts_with("store migration progress"))
        .collect();
    assert_eq!(progress.len(), 9, "one line per batch: {events:#?}");
    for line in &progress {
        for field in FIELDS {
            assert!(line.fields.contains_key(field), "{field} missing: {line:?}");
        }
    }

    let r = Root::new();
    write_legacy(&r.l, &small_fixture());
    let (opened, events) = open_logged(&r.location(), &MigrationIo::default());
    drop(opened.expect("the migration"));
    assert_eq!(
        events
            .iter()
            .filter(|e| e.message.starts_with("store migration progress"))
            .count(),
        0,
        "{events:#?}"
    );
    assert_eq!(count_lines(&events, tracing::Level::INFO, "migrating"), 1);
    assert_eq!(
        count_lines(&events, tracing::Level::INFO, "migration complete"),
        1
    );
}

#[test]
fn m17_store_location_from_the_environment() {
    let p = |s: &str| PathBuf::from(s);
    let os = |s: &'static str| Some(std::ffi::OsStr::new(s));
    let cases = [
        (
            None,
            None,
            (
                p("./topgun-data"),
                DataDirSource::Default,
                p("./topgun.redb"),
            ),
        ),
        (
            None,
            os("/x/a.redb"),
            (
                p("/x/topgun-data"),
                DataDirSource::DerivedFromRedbPath,
                p("/x/a.redb"),
            ),
        ),
        (
            None,
            os("a.redb"),
            (
                p("./topgun-data"),
                DataDirSource::DerivedFromRedbPath,
                p("./a.redb"),
            ),
        ),
        (
            os("/d"),
            None,
            (p("/d"), DataDirSource::TopgunDataDir, p("./topgun.redb")),
        ),
        (
            os("/d"),
            os("/x/a.redb"),
            (p("/d"), DataDirSource::TopgunDataDir, p("/x/a.redb")),
        ),
    ];
    for (data_dir, redb_path, (d, source, l)) in cases {
        let location = StoreLocation::from_vars(data_dir, redb_path)
            .unwrap_or_else(|e| panic!("({data_dir:?}, {redb_path:?}): {e:#}"));
        assert_eq!(
            location,
            StoreLocation {
                data_dir: d,
                data_dir_source: source,
                legacy_path: Some(l),
            },
            "({data_dir:?}, {redb_path:?})"
        );
    }
    assert_eq!(DataDirSource::TopgunDataDir.to_string(), "TOPGUN_DATA_DIR");
    assert_eq!(
        DataDirSource::DerivedFromRedbPath.to_string(),
        "derived from TOPGUN_REDB_PATH"
    );
    assert_eq!(DataDirSource::Default.to_string(), "default");

    for (data_dir, redb_path, var) in [
        (os(""), None, "TOPGUN_DATA_DIR"),
        (None, os(""), "TOPGUN_REDB_PATH"),
        (os(""), os("/x/a.redb"), "TOPGUN_DATA_DIR"),
        (os("/d"), os(""), "TOPGUN_REDB_PATH"),
    ] {
        let Err(e) = StoreLocation::from_vars(data_dir, redb_path) else {
            panic!("({data_dir:?}, {redb_path:?}): an empty value must be refused");
        };
        assert!(
            error_text(&e).contains(&format!("store location: {var} is set but empty")),
            "{e:#}"
        );
    }
}

#[test]
fn m18_the_startup_line_names_the_data_directory_and_where_it_came_from() {
    let dir = tempdir().expect("tempdir");
    let root = dir.path();
    for sub in ["a", "b", "c"] {
        fs::create_dir(root.join(sub)).unwrap();
    }
    let from_data_dir = StoreLocation::from_vars(
        Some(root.join("a").join("data").as_os_str()),
        Some(root.join("a").join("topgun.redb").as_os_str()),
    )
    .expect("location");
    let derived =
        StoreLocation::from_vars(None, Some(root.join("b").join("topgun.redb").as_os_str()))
            .expect("location");
    // The default source names `./topgun-data`; the test does not open a
    // store in its working directory, so it builds that location with
    // absolute paths.
    let default = StoreLocation {
        data_dir: root.join("c").join("topgun-data"),
        data_dir_source: DataDirSource::Default,
        legacy_path: Some(root.join("c").join("topgun.redb")),
    };
    let cases = [
        (
            from_data_dir,
            root.join("a").join("data"),
            root.join("a").join("topgun.redb"),
            "TOPGUN_DATA_DIR",
        ),
        (
            derived,
            root.join("b").join("topgun-data"),
            root.join("b").join("topgun.redb"),
            "derived from TOPGUN_REDB_PATH",
        ),
        (
            default,
            root.join("c").join("topgun-data"),
            root.join("c").join("topgun.redb"),
            "default",
        ),
    ];
    for (location, d, l, source) in cases {
        let (opened, events) = open_logged(&location, &MigrationIo::default());
        drop(opened.expect("a fresh start opens"));
        let startup: Vec<(usize, &Event)> = events
            .iter()
            .enumerate()
            .filter(|(_, e)| e.message.starts_with("store location: data directory"))
            .collect();
        assert_eq!(startup.len(), 1, "{source}: {events:#?}");
        let (index, line) = startup[0];
        assert_eq!(line.level, tracing::Level::INFO, "{source}");
        assert_eq!(
            line.message,
            format!("store location: data directory {}", d.display()),
            "{source}"
        );
        assert_eq!(
            line.field("data_dir"),
            Some(d.display().to_string()),
            "{source}"
        );
        assert_eq!(
            line.field("data_dir_source"),
            Some(source.to_string()),
            "{source}"
        );
        assert_eq!(
            line.field("legacy_path"),
            Some(l.display().to_string()),
            "{source}"
        );
        let first_store_line = events
            .iter()
            .position(Event::is_store_line)
            .expect("a store line");
        assert_eq!(
            index, first_store_line,
            "{source}: the startup line is first"
        );
    }

    let (opened, events) = captured(|| RedbDataStore::new(root.join("d")));
    drop(opened.expect("new opens"));
    assert!(
        !events
            .iter()
            .any(|e| e.message.starts_with("store location: data directory")),
        "new(path) logs no startup line: {events:#?}"
    );
}

#[test]
fn m19_the_stub_is_never_a_migration_source() {
    let elsewhere = PathBuf::from("/somewhere/else/topgun-data");
    for own in [true, false] {
        let label = if own {
            "own stub"
        } else {
            "stub naming another directory"
        };
        let stub_for = |r: &Root| {
            if own {
                stub_text(&r.d)
            } else {
                stub_text(&elsewhere)
            }
        };

        // (a) no store: refused, nothing created.
        let r = Root::new();
        fs::write(&r.l, stub_for(&r)).unwrap();
        let before = tree(&r.root, &r.d);
        let (opened, events) = open_logged(&r.location(), &MigrationIo::default());
        assert_refused(
            opened,
            &events,
            &format!(
                "store location: {} says the store was moved to a data directory, but {} holds no store",
                r.l.display(),
                r.d.display()
            ),
        );
        assert!(!exists(&r.d), "{label}: the data directory was created");
        assert!(
            !events.iter().any(|e| e.text().contains("migrating")),
            "{label}: {events:#?}"
        );
        assert_eq!(tree(&r.root, &r.d), before, "{label}: the tree changed");

        // (b) a store: opened, no warning, no migration, the stub unchanged.
        let r = Root::new();
        make_store_dir(&r.d);
        let stub = stub_for(&r);
        fs::write(&r.l, &stub).unwrap();
        let (opened, events) = open_logged(&r.location(), &MigrationIo::default());
        drop(opened.unwrap_or_else(|e| panic!("{label}: the store must open: {e:#}")));
        assert!(
            // Store lines only: a fault-injection build warns at every open.
            !events
                .iter()
                .any(|e| e.level == tracing::Level::WARN && e.is_store_line()),
            "{label}: {events:#?}"
        );
        assert!(
            !events
                .iter()
                .any(|e| e.is_store_line() && e.text().contains("migrat")),
            "{label}: {events:#?}"
        );
        assert_eq!(fs::read(&r.l).expect("read stub"), stub, "{label}");
    }

    // (d) the stub never begins with redb's magic.
    assert_eq!(REDB_FILE_MAGIC, REDB_MAGIC, "the magic constant");
    let r = Root::new();
    assert!(!stub_text(&r.d).starts_with(&REDB_FILE_MAGIC));
    drop(RedbDataStore::open_location(&r.location()).expect("a fresh start"));
    assert!(!fs::read(&r.l)
        .expect("the written stub")
        .starts_with(&REDB_FILE_MAGIC));
}

#[test]
fn m20_a_store_with_a_non_store_file_at_the_old_path_warns_once_and_opens() {
    let r = Root::new();
    make_store_dir(&r.d);
    fs::write(&r.l, vec![0u8; 4096]).unwrap();
    let bytes = fs::read(&r.l).unwrap();
    let line = format!(
        "store location: TOPGUN_REDB_PATH names {}, which is not read: TOPGUN_DATA_DIR ({}) already holds a store",
        r.l.display(),
        r.d.display()
    );
    for start in 1..=2 {
        let (opened, events) = open_logged(&r.location(), &MigrationIo::default());
        drop(opened.unwrap_or_else(|e| panic!("start {start}: the store must open: {e:#}")));
        assert_eq!(
            count_lines(&events, tracing::Level::WARN, &line),
            1,
            "start {start}: {events:#?}"
        );
        assert_eq!(fs::read(&r.l).unwrap(), bytes, "start {start}: L changed");
    }
}

#[test]
fn m21_a_store_with_an_empty_file_at_the_old_path_is_refused_and_touches_nothing() {
    let r = Root::new();
    make_store_dir(&r.d);
    fs::write(&r.l, b"").unwrap();
    let dump_before = dump_engine_file(&r.engine_path());
    let before = tree(&r.root, &r.d);

    let (opened, events) = open_logged(&r.location(), &MigrationIo::default());
    let Err(e) = opened else {
        panic!("a zero-length file at the old path must be refused");
    };
    let text = error_text(&e);
    assert!(
        text.contains("is an empty file") && text.contains(&r.l.display().to_string()),
        "{text}"
    );
    assert!(
        !events
            .iter()
            .any(|e| e.text().contains("which is not read")),
        "no L4 warning: {events:#?}"
    );
    let meta = fs::symlink_metadata(&r.l).expect("L still exists");
    assert!(
        meta.is_file() && meta.len() == 0,
        "L is still a zero-length file"
    );
    assert!(
        dump_engine_file(&r.engine_path()) == dump_before,
        "the engine's content changed"
    );
    assert_eq!(tree(&r.root, &r.d), before, "the tree changed");

    // Control: one byte is not an empty file, so it is the L4 warning.
    fs::write(&r.l, b"x").unwrap();
    let (opened, events) = open_logged(&r.location(), &MigrationIo::default());
    drop(opened.unwrap_or_else(|e| panic!("a 1-byte file is warned about, not refused: {e:#}")));
    assert_eq!(
        count_lines(&events, tracing::Level::WARN, "which is not read"),
        1,
        "{events:#?}"
    );
}

#[test]
fn m22_the_migration_holds_the_engine_exclusively_until_the_store_is_open() {
    use std::cell::RefCell;
    use std::rc::Rc;

    let r = Root::new();
    let fixture = small_fixture();
    write_legacy(&r.l, &fixture);
    let location = r.location();
    let engine_path = r.engine_path();

    let at_step_5: Rc<RefCell<Option<Result<(), String>>>> = Rc::new(RefCell::new(None));
    let slot = Rc::clone(&at_step_5);
    let probe_path = engine_path.clone();
    let io = MigrationIo {
        after_step: Some(Box::new(move |step| {
            if step == 5 {
                let verdict = match RedbKvEngine::open(&probe_path) {
                    Ok(engine) => {
                        drop(engine);
                        Err("a second open succeeded while the migration ran".to_string())
                    }
                    Err(e) if kv_says_already_open(&e) => Ok(()),
                    Err(e) => Err(format!("a second open failed otherwise: {e}")),
                };
                *slot.borrow_mut() = Some(verdict);
            }
        })),
        ..MigrationIo::default()
    };

    let store = RedbDataStore::open_location_with(&location, &io).expect("the migration");
    assert_eq!(
        at_step_5.borrow().clone(),
        Some(Ok(())),
        "an open between steps 5 and 6 meets the migration's engine"
    );

    match RedbDataStore::open_location(&location) {
        Ok(_) => panic!("a second store opened beside the first"),
        Err(e) => assert!(
            first_kv_error(&e).is_some_and(kv_says_already_open),
            "a second open_location is refused as already open: {e:#}"
        ),
    }
    match RedbKvEngine::open(&engine_path) {
        Ok(_) => panic!("a second engine opened beside the store"),
        Err(e) => assert!(kv_says_already_open(&e), "{e}"),
    }

    drop(store);
    let engine =
        Arc::new(RedbKvEngine::open(&engine_path).expect("open after the store is dropped"));
    assert_eq!(
        EngineMapDataStore::<RedbKvEngine>::read_migration_mark(&engine).expect("mark"),
        None,
        "the migration completed"
    );
    let store = EngineMapDataStore::open(engine).expect("the migrated store opens");
    assert_holds_fixture(&store, &fixture);
}

// ---------------------------------------------------------------------------
// Measurements
// ---------------------------------------------------------------------------

/// The storage-level performance gate's measurement. Public API only, so the
/// same fn compiles pasted into an older tree's store test module. Prints one
/// `M90 metric=… median_ns=…` line per metric (median of 5 repeats).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "measurement: run with --ignored --nocapture"]
#[allow(clippy::too_many_lines)]
async fn m90_storage_gate_measurement() {
    use crate::storage::datastores::{RedbDataStore, WriteBehindConfig, WriteBehindDataStore};
    use crate::storage::map_data_store::{LeafSink, MapDataStore, MerkleLeaf};
    use crate::storage::record::RecordValue;
    use std::sync::Arc;
    use std::time::{Duration, Instant};
    use topgun_core::hlc::Timestamp;
    use topgun_core::types::Value;

    const REPEATS: usize = 5;
    const DRAIN_DEADLINE: Duration = Duration::from_secs(1_800);

    struct CountingSink(u64);
    #[async_trait::async_trait]
    impl LeafSink for CountingSink {
        async fn consume(&mut self, batch: Vec<MerkleLeaf>) -> anyhow::Result<()> {
            self.0 += batch.len() as u64;
            Ok(())
        }
    }

    // About 100 bytes once encoded.
    fn value(tag: u64) -> RecordValue {
        RecordValue::Lww {
            value: Value::String(format!("{tag:08}{}", "v".repeat(60))),
            timestamp: Timestamp {
                millis: 1_000 + tag,
                counter: 0,
                node_id: "gate".to_string(),
            },
        }
    }

    fn median(samples: &mut [u128]) -> u128 {
        samples.sort_unstable();
        samples[samples.len() / 2]
    }

    fn report(metric: &str, samples: &mut [u128]) {
        let repeats = samples.to_vec();
        println!(
            "M90 metric={metric} median_ns={} repeats_ns={repeats:?}",
            median(samples)
        );
    }

    async fn drained(store: &WriteBehindDataStore) {
        let deadline = Instant::now() + DRAIN_DEADLINE;
        while store.pending_operation_count() > 0 {
            assert!(
                Instant::now() < deadline,
                "the flush did not drain within {DRAIN_DEADLINE:?}"
            );
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
    }

    async fn full_walk(store: &dyn MapDataStore, map: &str, budget: u64) -> u64 {
        let mut rows = 0u64;
        let mut batch = store.scan_values(map, false, budget).await.expect("scan");
        loop {
            rows += batch.records.len() as u64;
            match batch.next_cursor.take() {
                Some(cursor) => {
                    batch = store
                        .scan_values_batched(map, false, cursor, budget)
                        .await
                        .expect("scan batch");
                }
                None => break,
            }
        }
        rows
    }

    let dir = tempfile::tempdir().expect("tempdir");
    let store = Arc::new(RedbDataStore::new(dir.path().join("gate-store")).expect("store"));
    let inner: Arc<dyn MapDataStore> = store.clone();

    // W1, W2: the write-behind flush path, one store write per entry.
    let (mut w1, mut w2) = (Vec::new(), Vec::new());
    for rep in 0..REPEATS {
        let wb = WriteBehindDataStore::new(
            Arc::clone(&inner),
            WriteBehindConfig {
                write_delay_ms: 0,
                flush_interval_ms: 3_600_000,
                capacity: 0,
                ..WriteBehindConfig::default()
            },
        );
        for (pass, samples) in [(1u64, &mut w1), (2u64, &mut w2)] {
            for i in 0..20_000u64 {
                wb.add(
                    &format!("w{rep}_{}", i % 10),
                    &format!("k{i:05}"),
                    &value(pass * 100_000 + i),
                    0,
                    1_000,
                )
                .await
                .expect("add");
            }
            let started = Instant::now();
            wb.soft_flush().await.expect("soft flush");
            drained(&wb).await;
            samples.push(started.elapsed().as_nanos());
        }
    }
    report("W1", &mut w1);
    report("W2", &mut w2);

    // R1: loads of existing keys.
    let mut r1 = Vec::new();
    for _ in 0..REPEATS {
        let started = Instant::now();
        for i in 0..20_000u64 {
            let got = store
                .load(&format!("w0_{}", i % 10), &format!("k{i:05}"))
                .await
                .expect("load");
            assert!(got.is_some());
        }
        r1.push(started.elapsed().as_nanos());
    }
    report("R1", &mut r1);

    // R2, R3: a 100 000-key map; R4: a 10-key map.
    let populated = Instant::now();
    for i in 0..100_000u64 {
        store
            .add("big", &format!("b{i:06}"), &value(i), 0, 1_000)
            .await
            .expect("populate");
    }
    for i in 0..10u64 {
        store
            .add("small", &format!("s{i:02}"), &value(i), 0, 1_000)
            .await
            .expect("populate");
    }
    println!(
        "M90 populate=big+small elapsed_ms={}",
        populated.elapsed().as_millis()
    );
    let (mut r2, mut r3, mut r4) = (Vec::new(), Vec::new(), Vec::new());
    for _ in 0..REPEATS {
        let started = Instant::now();
        assert_eq!(full_walk(store.as_ref(), "big", 0).await, 100_000);
        r2.push(started.elapsed().as_nanos());

        let started = Instant::now();
        let mut sink = CountingSink(0);
        store
            .enumerate_leaves("big", false, &mut sink)
            .await
            .expect("leaves");
        assert_eq!(sink.0, 100_000);
        r3.push(started.elapsed().as_nanos());

        let started = Instant::now();
        assert_eq!(full_walk(store.as_ref(), "small", 0).await, 10);
        r4.push(started.elapsed().as_nanos());
    }
    report("R2", &mut r2);
    report("R3", &mut r3);
    report("R4", &mut r4);

    // R5: 1 000 one-key maps, in a store of their own.
    let r5_dir = tempfile::tempdir().expect("tempdir");
    let r5_store = RedbDataStore::new(r5_dir.path().join("gate-store")).expect("store");
    for i in 0..1_000u64 {
        r5_store
            .add(&format!("one{i:04}"), "k", &value(i), 0, 1_000)
            .await
            .expect("populate");
    }
    let mut r5 = Vec::new();
    for _ in 0..REPEATS {
        let started = Instant::now();
        let maps = r5_store.list_maps().await.expect("list_maps");
        assert_eq!(maps.len(), 1_000);
        let mut sink = CountingSink(0);
        for map in &maps {
            r5_store
                .enumerate_leaves(map, false, &mut sink)
                .await
                .expect("leaves");
        }
        assert_eq!(sink.0, 1_000);
        r5.push(started.elapsed().as_nanos());
    }
    report("R5", &mut r5);

    // W3: a full walk with a small budget through a full write-behind buffer.
    let w3_dir = tempfile::tempdir().expect("tempdir");
    let w3_store: Arc<dyn MapDataStore> =
        Arc::new(RedbDataStore::new(w3_dir.path().join("gate-store")).expect("store"));
    let mut w3 = Vec::new();
    for rep in 0..REPEATS {
        let map = format!("w3_{rep}");
        for i in 0..10_000u64 {
            w3_store
                .add(&map, &format!("k{i:05}"), &value(i), 0, 1_000)
                .await
                .expect("durable key");
        }
        let wb = WriteBehindDataStore::new(
            Arc::clone(&w3_store),
            WriteBehindConfig {
                write_delay_ms: 3_600_000,
                flush_interval_ms: 3_600_000,
                capacity: 10_000,
                ..WriteBehindConfig::default()
            },
        );
        for i in 0..5_000u64 {
            wb.add(&map, &format!("k{i:05}"), &value(500_000 + i), 0, 2_000)
                .await
                .expect("buffered overwrite");
        }
        for i in 10_000..15_000u64 {
            wb.add(&map, &format!("k{i:05}"), &value(i), 0, 2_000)
                .await
                .expect("buffered new key");
        }
        let started = Instant::now();
        assert_eq!(full_walk(wb.as_ref(), &map, 1_024).await, 15_000);
        w3.push(started.elapsed().as_nanos());
    }
    report("W3", &mut w3);
}

/// The migration of a legacy file of about 1 GiB: prints its duration, rows
/// per second and the size of the data directory over the legacy file's.
#[test]
#[ignore = "measurement: run with --ignored --nocapture"]
#[allow(clippy::cast_precision_loss)]
fn m91_migration_of_one_gibibyte() {
    const MAPS: usize = 100;
    const ROWS: usize = 10_000;

    fn dir_size(path: &Path) -> u64 {
        let meta = fs::symlink_metadata(path).expect("metadata");
        if meta.is_dir() {
            fs::read_dir(path)
                .expect("read_dir")
                .map(|e| dir_size(&e.expect("entry").path()))
                .sum()
        } else {
            meta.len()
        }
    }

    let r = Root::new();
    {
        let db = redb::Database::create(&r.l).expect("legacy create");
        for m in 0..MAPS {
            let txn = db.begin_write().expect("begin_write");
            {
                let name = table_name_for(&format!("map{m:03}"), false).expect("name");
                let mut t = txn.open_table(table_def(&name)).expect("open table");
                for i in 0..ROWS {
                    let mut value = format!("{m:03}-{i:05}-").into_bytes();
                    value.resize(1_024, b'0' + u8::try_from(i % 10).unwrap_or(0));
                    t.insert(format!("k{i:05}").as_str(), value.as_slice())
                        .expect("insert");
                }
            }
            txn.commit().expect("commit");
        }
    }
    let legacy_size = fs::metadata(&r.l).expect("legacy size").len();

    let started = Instant::now();
    let store = RedbDataStore::open_location(&r.location()).expect("the migration");
    let elapsed = started.elapsed();
    drop(store);

    let rows = (MAPS * ROWS) as f64;
    let data_dir_size = dir_size(&r.d);
    println!(
        "M91 elapsed_ms={} rows_per_s={:.0} size_ratio={:.3} data_dir_bytes={data_dir_size} legacy_bytes={legacy_size}",
        elapsed.as_millis(),
        rows / elapsed.as_secs_f64(),
        data_dir_size as f64 / legacy_size as f64
    );
}
