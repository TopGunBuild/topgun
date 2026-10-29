//! A store file redb 2 panics on at open is refused like any other store that
//! does not open: the server exits 1 with one ERROR line naming the file
//! (`TG-ENG-001`).
//!
//! Two files, each a cleanly closed `engine.redb` built fresh for every run:
//!
//! * cut short by a few pages, so its header describes a layout longer than
//!   the file — the state a crash leaves when the header with a grown layout
//!   reached the disk and the file's growth did not. redb asserts on it before
//!   it writes anything, so the file stays byte for byte as it was;
//! * lengthened by a part of a page, a length redb never gives its file. redb
//!   rewrites the header before it asserts, so only the refusal is claimed,
//!   and that a second start refuses the same way.

#![cfg(feature = "redb")]

use std::io::Read as _;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const TABLE: redb::TableDefinition<&[u8], &[u8]> = redb::TableDefinition::new("t");
const EXIT_WITHIN: Duration = Duration::from_secs(60);
const PAGE: i64 = 4096;

/// Writes a cleanly closed redb file at `path` and moves its end by `delta` bytes.
fn engine_file_with_length_moved(path: &Path, delta: i64) {
    let db = redb::Database::create(path).expect("create");
    let txn = db.begin_write().expect("begin");
    {
        let mut table = txn.open_table(TABLE).expect("table");
        for i in 0u32..2_000 {
            table
                .insert(i.to_be_bytes().as_slice(), [7u8; 1024].as_slice())
                .expect("insert");
        }
    }
    txn.commit().expect("commit");
    drop(db);
    let file = std::fs::OpenOptions::new()
        .write(true)
        .open(path)
        .expect("reopen");
    let len = file.metadata().expect("metadata").len();
    file.set_len(len.checked_add_signed(delta).expect("length"))
        .expect("move the end");
}

/// One server start on `data_dir`: its exit code (`None` when it had not
/// exited within the deadline and was killed) and everything it printed.
fn start(root: &Path, data_dir: &Path) -> (Option<i32>, String) {
    let mut child = Command::new(env!("CARGO_BIN_EXE_topgun-server"))
        .env_clear()
        .current_dir(root)
        .env("TOPGUN_NO_AUTH", "1")
        .env("PORT", "0")
        .env("TOPGUN_DATA_DIR", data_dir)
        .env("TOPGUN_WAL_DIR", root.join("wal"))
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("the server starts");
    let started = Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait().expect("wait") {
            break Some(status);
        }
        if started.elapsed() > EXIT_WITHIN {
            child.kill().expect("kill");
            child.wait().expect("reap");
            break None;
        }
        std::thread::sleep(Duration::from_millis(50));
    };
    let mut output = String::new();
    for stream in [
        Box::new(child.stdout.take().expect("stdout")) as Box<dyn std::io::Read>,
        Box::new(child.stderr.take().expect("stderr")),
    ] {
        let mut stream = stream;
        stream.read_to_string(&mut output).expect("output");
    }
    (status.and_then(|s| s.code()), output)
}

/// The one refusal line a start logged at ERROR for `engine`.
fn refusal_line(engine: &Path, code: Option<i32>, output: &str) -> String {
    let refusal = format!(
        "store open: {} does not open with this version: redb panicked while opening it",
        engine.display()
    );
    let lines: Vec<&str> = output
        .lines()
        .filter(|line| line.contains("ERROR") && line.contains(&refusal))
        .collect();
    assert_eq!(
        (code, lines.len()),
        (Some(1), 1),
        "exit 1 within {EXIT_WITHIN:?} and one ERROR line naming the file; output:\n{output}"
    );
    assert!(
        lines[0].contains("do not delete the file")
            && lines[0].contains("restore the data directory and the WAL directory"),
        "the line says what to do: {}",
        lines[0]
    );
    lines[0]
        .split_once("store open:")
        .expect("line")
        .1
        .to_string()
}

#[test]
fn a_store_file_shorter_than_its_layout_is_refused_with_exit_1_and_left_unchanged() {
    let dir = tempfile::tempdir().expect("tempdir");
    let data_dir = dir.path().join("data");
    std::fs::create_dir(&data_dir).expect("data dir");
    let engine = data_dir.join("engine.redb");
    engine_file_with_length_moved(&engine, -4 * PAGE);
    let before = std::fs::read(&engine).expect("read before");

    let (code, output) = start(dir.path(), &data_dir);
    refusal_line(&engine, code, &output);
    assert!(
        output.contains("page_manager.rs:243"),
        "the panic's place in redb stays in the output for diagnosis; output:\n{output}"
    );
    assert_eq!(
        std::fs::read(&engine).expect("read after"),
        before,
        "the refused file is left byte for byte as it was"
    );
}

#[test]
fn a_store_file_of_a_length_redb_never_gives_is_refused_the_same_way_at_every_start() {
    let dir = tempfile::tempdir().expect("tempdir");
    let data_dir = dir.path().join("data");
    std::fs::create_dir(&data_dir).expect("data dir");
    let engine = data_dir.join("engine.redb");
    engine_file_with_length_moved(&engine, PAGE - 512);

    let (code, output) = start(dir.path(), &data_dir);
    let first = refusal_line(&engine, code, &output);
    let (code, output) = start(dir.path(), &data_dir);
    let second = refusal_line(&engine, code, &output);
    assert_eq!(first, second, "a second start refuses with the same line");
}
