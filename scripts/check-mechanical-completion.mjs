#!/usr/bin/env node
// Mechanical-completion filter: proves that every change outside a part's
// counted files is a purely mechanical completion of a type or signature
// change, so a reviewer only has to read the counted files.
//
// Usage:
//   node scripts/check-mechanical-completion.mjs --base <ref> --head <ref> \
//       --profile ttl-field|g1|g2 [--counted <path>...]
//   node scripts/check-mechanical-completion.mjs --self-test
//
// Every path given to --counted (repo-relative) is skipped entirely, whether
// the diff adds it or modifies it: counted files are reviewed by hand and are
// not limited by this filter. For every other file changed between base and
// head:
//   - the file must be modified in place: an added, deleted, renamed, copied
//     or type-changed file is rejected;
//   - every hunk (git's default three lines of context) is reduced to its old side
//     R and its new side A, with all whitespace removed so that a rustfmt
//     reflow compares equal, and the profile decides whether R -> A is
//     mechanical.
//
// Profile ttl-field: strip `ttl_ms:None,?` and `,?\.\.(?=\})` from both sides,
// then turn a trailing comma before a closing delimiter (`,)` / `,}`) into the
// delimiter alone; the results must be equal, and A must hold at least as
// many `ttl_ms:None` and `..}` as R. The trailing-comma step is needed because
// rustfmt writes `ttl_ms: None` as the last field of a one-line literal
// without a trailing comma, and `..` on its own line after `timestamp,`.
//
// Profiles g1 / g2: drop from R the removed lines that are `//` comments
// naming a removed identifier; strip from R every match of the profile's
// regexes; normalise both sides (a single-item brace group after a path
// separator, `::{X}` / `::{X,}`, becomes `::X`, and `,)` / `,}` become `)` /
// `}`); the results must be equal and A must contain none of the regexes.
//
// Exit code 0: every hunk accepted. 1: at least one file or hunk rejected
// (each is printed). 2: usage or git error.

import { execFileSync } from 'node:child_process';
import { mkdtempSync, rmSync, writeFileSync, mkdirSync, renameSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join, dirname } from 'node:path';

const PROFILES = {
  'ttl-field': { kind: 'ttl' },
  g1: {
    kind: 'strip',
    identifiers: ['ExpiryPolicy', 'ExpiryReason', 'expiry', 'default_ttl_millis', 'default_max_idle_millis'],
    regexes: [
      /ExpiryPolicy::NONE,?/g,
      /,?_?expiry:&?([A-Za-z_]+::)*ExpiryPolicy/g,
      /,expiry(?=[,)])/g,
      /use(crate|super|self)?(::[a-z_]+)*::(ExpiryPolicy|ExpiryReason);/g,
      /([a-z_]+::)*(ExpiryPolicy|ExpiryReason),?/g,
      /default_ttl_millis:\d+,?/g,
      /default_max_idle_millis:\d+,?/g,
    ],
    gated: null,
  },
  g2: {
    kind: 'strip',
    identifiers: ['expiration_time'],
    regexes: [
      /,?_?expiration_time:(i64|u64|Option<i64>|Option<u64>)/g,
      /,\*?expiration_time(?=[,)])/g,
      /expiration_time(:[A-Za-z0-9_*]+)?,/g,
    ],
    // Applied only to hunks whose R names one of the functions or variants
    // that lose the argument; the compiler rejects the deletion anywhere else.
    gated: {
      triggers: ['add(', 'add_encoded(', 'add_backup(', 'Put{', 'Store{'],
      regex: /,(0|0i64|None)(?=[,)])/g,
    },
  },
};

const noWs = (s) => s.replace(/\s+/g, '');
const count = (s, needle) => s.split(needle).length - 1;
const trailingComma = (s) => s.replace(/,\)/g, ')').replace(/,\}/g, '}');
const singleItemGroup = (s) => s.replace(/::\{([^{},]*),?\}/g, '::$1');

// A hunk is a list of lines { kind: ' ' | '-' | '+', text }.
export function checkHunk(profileName, lines) {
  const profile = PROFILES[profileName];
  if (!profile) throw new Error(`unknown profile ${profileName}`);
  if (profile.kind === 'ttl') {
    const r = noWs(lines.filter((l) => l.kind !== '+').map((l) => l.text).join('\n'));
    const a = noWs(lines.filter((l) => l.kind !== '-').map((l) => l.text).join('\n'));
    const strip = (s) => trailingComma(s.replace(/ttl_ms:None,?/g, '').replace(/,?\.\.(?=\})/g, ''));
    if (strip(r) !== strip(a)) return 'old and new side differ beyond `ttl_ms: None` and `..`';
    if (count(a, 'ttl_ms:None') < count(r, 'ttl_ms:None')) return 'the hunk removes a `ttl_ms: None`';
    if (count(a, '..}') < count(r, '..}')) return 'the hunk removes a `..`';
    return null;
  }
  const namesRemoved = (text) =>
    /^\s*\/\//.test(text) && profile.identifiers.some((id) => new RegExp(`\\b${id}\\b`).test(text));
  let r = noWs(
    lines
      .filter((l) => l.kind === ' ' || (l.kind === '-' && !namesRemoved(l.text)))
      .map((l) => l.text)
      .join('\n'),
  );
  const a = noWs(lines.filter((l) => l.kind !== '-').map((l) => l.text).join('\n'));
  const regexes = [...profile.regexes];
  if (profile.gated && profile.gated.triggers.some((t) => r.includes(t))) regexes.push(profile.gated.regex);
  for (const re of regexes) r = r.replace(re, '');
  const norm = (s) => trailingComma(singleItemGroup(s));
  if (norm(r) !== norm(a)) return 'old side minus the removed items differs from the new side';
  for (const re of regexes) {
    re.lastIndex = 0;
    if (re.test(a)) return `the new side still matches ${re}`;
    re.lastIndex = 0;
  }
  return null;
}

function git(cwd, args) {
  return execFileSync('git', args, { cwd, encoding: 'utf8', maxBuffer: 1 << 30 });
}

function parseHunks(diffText) {
  const hunks = [];
  let current = null;
  for (const line of diffText.split('\n')) {
    if (line.startsWith('@@')) {
      current = { header: line, lines: [] };
      hunks.push(current);
    } else if (current && (line.startsWith(' ') || line.startsWith('-') || line.startsWith('+'))) {
      current.lines.push({ kind: line[0], text: line.slice(1) });
    } else if (current && line.startsWith('\\')) {
      // "\ No newline at end of file" carries no text.
    }
  }
  return hunks;
}

// Returns { rejected: [...], files, hunks, skipped }.
export function checkRange({ cwd, base, head, profile, counted }) {
  const countedSet = new Set(counted);
  const rejected = [];
  let files = 0;
  let hunkCount = 0;
  const skipped = [];
  const status = git(cwd, ['diff', '--no-ext-diff', '--name-status', '-M', '-C', base, head, '--']);
  for (const row of status.split('\n').filter(Boolean)) {
    const parts = row.split('\t');
    const code = parts[0];
    const paths = parts.slice(1);
    if (paths.some((p) => countedSet.has(p))) {
      skipped.push(`${code}\t${paths.join(' -> ')}`);
      continue;
    }
    if (code !== 'M') {
      rejected.push({ file: paths.join(' -> '), reason: `status ${code}: only in-place modification is allowed outside counted files` });
      continue;
    }
    files += 1;
    const path = paths[0];
    const diff = git(cwd, ['diff', '--no-ext-diff', '--no-color', '-U3', base, head, '--', path]);
    if (/^Binary files /m.test(diff)) {
      rejected.push({ file: path, reason: 'binary change' });
      continue;
    }
    for (const hunk of parseHunks(diff)) {
      hunkCount += 1;
      const reason = checkHunk(profile, hunk.lines);
      if (reason) rejected.push({ file: path, header: hunk.header, reason, lines: hunk.lines });
    }
  }
  return { rejected, files, hunks: hunkCount, skipped };
}

function printRejected(rejected) {
  for (const r of rejected) {
    console.log(`REJECTED ${r.file}${r.header ? ` ${r.header}` : ''}: ${r.reason}`);
    for (const l of r.lines ?? []) console.log(`  ${l.kind}${l.text}`);
  }
}

// ---------------------------------------------------------------- self-test

const hunk = (text) =>
  text
    .split('\n')
    .filter((l) => l.length > 0)
    .map((l) => ({ kind: l[0], text: l.slice(1) }));

const FIXTURES = [
  // ttl-field, accepted
  ['ttl-field', true, 'literal gains a last field on one line', hunk(`
-    let v = RecordValue::Lww { value: Value::Int(1), timestamp: ts };
+    let v = RecordValue::Lww { value: Value::Int(1), timestamp: ts, ttl_ms: None };
`)],
  ['ttl-field', true, 'multi-line literal gains a field line', hunk(`
         timestamp: ts.clone(),
+        ttl_ms: None,
     };
`)],
  ['ttl-field', true, 'rustfmt reflows a literal that grew too long', hunk(`
-    store.put("k", RecordValue::Lww { value: Value::String("abc".into()), timestamp: ts }, p);
+    store.put(
+        "k",
+        RecordValue::Lww {
+            value: Value::String("abc".into()),
+            timestamp: ts,
+            ttl_ms: None,
+        },
+        p,
+    );
`)],
  ['ttl-field', true, 'one-line pattern gains ..', hunk(`
-    RecordValue::Lww { value, timestamp } => (value, timestamp),
+    RecordValue::Lww { value, timestamp, .. } => (value, timestamp),
`)],
  ['ttl-field', true, 'multi-line pattern gains a .. line', hunk(`
         timestamp,
+        ..
     } => {
`)],
  ['ttl-field', true, 'OrMapEntry literal gains a middle field', hunk(`
-    OrMapEntry { value: v, tag: t, timestamp: ts }
+    OrMapEntry { value: v, tag: t, timestamp: ts, ttl_ms: None }
`)],
  // ttl-field, rejected
  ['ttl-field', false, 'value changed next to the new field', hunk(`
-    let v = RecordValue::Lww { value: Value::Int(1), timestamp: ts };
+    let v = RecordValue::Lww { value: Value::Int(2), timestamp: ts, ttl_ms: None };
`)],
  ['ttl-field', false, 'TTL set to Some', hunk(`
         timestamp: ts.clone(),
+        ttl_ms: Some(5),
     };
`)],
  ['ttl-field', false, 'existing ttl_ms: None removed', hunk(`
         timestamp: ts.clone(),
-        ttl_ms: None,
     };
`)],
  ['ttl-field', false, 'existing .. removed', hunk(`
-    RecordValue::Lww { value, timestamp, .. } => (value, timestamp),
+    RecordValue::Lww { value, timestamp } => (value, timestamp),
`)],
  ['ttl-field', false, 'unrelated statement added', hunk(`
     let a = 1;
+    let b = 2;
     let c = 3;
`)],
  // g1, accepted
  ['g1', true, 'argument deleted from a one-line call', hunk(`
-    store.put("k", value, ExpiryPolicy::NONE, CallerProvenance::Client);
+    store.put("k", value, CallerProvenance::Client);
`)],
  ['g1', true, 'grouped import loses one item', hunk(`
-use crate::storage::record_store::{CallerProvenance, ExpiryPolicy};
+use crate::storage::record_store::CallerProvenance;
`)],
  ['g1', true, 'trailing argument deleted from a multi-line call', hunk(`
         CallerProvenance::Client,
-        ExpiryPolicy::NONE,
     )
`)],
  ['g1', true, 'parameter deleted from an impl signature and forwarded argument', hunk(`
-    fn put(&self, key: &str, value: RecordValue, expiry: ExpiryPolicy, p: CallerProvenance) -> R {
-        self.inner.put(key, value, expiry, p)
+    fn put(&self, key: &str, value: RecordValue, p: CallerProvenance) -> R {
+        self.inner.put(key, value, p)
`)],
  ['g1', true, 'comment naming the removed identifier deleted', hunk(`
     let p = CallerProvenance::Client;
-    // ExpiryPolicy::NONE because client writes never expire here.
-    store.put("k", value, ExpiryPolicy::NONE, p);
+    store.put("k", value, p);
`)],
  // g1, rejected
  ['g1', false, 'another argument changed with the deletion', hunk(`
-    store.put("k", value, ExpiryPolicy::NONE, CallerProvenance::Client);
+    store.put("k", other, CallerProvenance::Client);
`)],
  ['g1', false, 'new side still names the removed type', hunk(`
-    store.put("k", value, ExpiryPolicy::NONE, p);
+    store.put("k", value, ExpiryPolicy::default(), p);
`)],
  // g2, accepted
  ['g2', true, 'literal 0 deleted from an add call', hunk(`
-    ds.add("m", "k", &v, 0, now).await?;
+    ds.add("m", "k", &v, now).await?;
`)],
  ['g2', true, 'grouped import loses one item', hunk(`
-use crate::storage::wal::{expiration_time, WalOp};
+use crate::storage::wal::WalOp;
`)],
  ['g2', true, 'trailing argument deleted from a multi-line add call', hunk(`
     ds.add(
         &key,
         &value,
-        0,
     )
`)],
  ['g2', true, 'field deleted from a Store variant literal', hunk(`
-    WalOp::Store { key: k, value: v, expiration_time: 0, seq };
+    WalOp::Store { key: k, value: v, seq };
`)],
  ['g2', true, 'parameter deleted from an impl signature and forwarded argument', hunk(`
-    async fn add(&self, map: &str, key: &str, v: &RecordValue, expiration_time: i64, now: i64) -> R {
-        self.inner.add(map, key, v, expiration_time, now).await
+    async fn add(&self, map: &str, key: &str, v: &RecordValue, now: i64) -> R {
+        self.inner.add(map, key, v, now).await
`)],
  // g2, rejected
  ['g2', false, 'literal 0 deleted from an unrelated call', hunk(`
-    let x = compute(a, 0);
+    let x = compute(a);
`)],
  ['g2', false, 'value changed together with the deletion', hunk(`
-    ds.add("m", "k", &v, 0, now).await?;
+    ds.add("m", "k", &w, now).await?;
`)],
];

function runGit(cwd, args, env = {}) {
  return execFileSync('git', args, {
    cwd,
    encoding: 'utf8',
    env: {
      ...process.env,
      GIT_AUTHOR_NAME: 'self-test',
      GIT_AUTHOR_EMAIL: 'self-test@localhost',
      GIT_COMMITTER_NAME: 'self-test',
      GIT_COMMITTER_EMAIL: 'self-test@localhost',
      GIT_CONFIG_GLOBAL: '/dev/null',
      GIT_CONFIG_NOSYSTEM: '1',
      ...env,
    },
  });
}

function selfTestRange() {
  const dir = mkdtempSync(join(tmpdir(), 'mech-completion-'));
  const failures = [];
  try {
    const write = (p, s) => {
      mkdirSync(dirname(join(dir, p)), { recursive: true });
      writeFileSync(join(dir, p), s);
    };
    runGit(dir, ['init', '-q', '-b', 'main']);
    write('a.rs', 'fn a() {\n    let v = RecordValue::Lww { value: x, timestamp: t };\n}\n');
    write('b.rs', 'fn b() {\n    let n = 1;\n}\n');
    write('c.rs', 'fn c() {\n    let n = 1;\n}\n');
    write('counted.rs', 'fn d() {}\n');
    write('gone.rs', 'fn e() {}\n');
    write('moved.rs', 'fn f() {\n    let a = 1;\n    let b = 2;\n    let c = 3;\n}\n');
    runGit(dir, ['add', '-A']);
    runGit(dir, ['commit', '-q', '-m', 'base']);
    const base = runGit(dir, ['rev-parse', 'HEAD']).trim();

    // Mechanical change in a.rs; counted file rewritten; new counted file.
    write('a.rs', 'fn a() {\n    let v = RecordValue::Lww { value: x, timestamp: t, ttl_ms: None };\n}\n');
    write('counted.rs', 'fn d() { anything_goes(); }\n');
    write('new_counted.rs', 'fn g() {}\n');
    runGit(dir, ['add', '-A']);
    runGit(dir, ['commit', '-q', '-m', 'ok']);
    const ok = runGit(dir, ['rev-parse', 'HEAD']).trim();
    const counted = ['counted.rs', 'new_counted.rs'];
    const good = checkRange({ cwd: dir, base, head: ok, profile: 'ttl-field', counted });
    if (good.rejected.length !== 0) failures.push(`range: a mechanical range was rejected: ${JSON.stringify(good.rejected)}`);
    if (good.skipped.length !== 2) failures.push(`range: expected 2 counted files skipped, got ${good.skipped.length}`);

    // Each forbidden kind of change, one per commit on top of `ok`.
    const forbidden = [
      ['non-mechanical hunk', () => write('b.rs', 'fn b() {\n    let n = 2;\n}\n')],
      ['added file', () => write('added.rs', 'fn h() {}\n')],
      ['deleted file', () => rmSync(join(dir, 'gone.rs'))],
      ['renamed file', () => renameSync(join(dir, 'moved.rs'), join(dir, 'renamed.rs'))],
    ];
    for (const [name, change] of forbidden) {
      runGit(dir, ['checkout', '-q', ok]);
      change();
      runGit(dir, ['add', '-A']);
      runGit(dir, ['commit', '-q', '-m', name]);
      const head = runGit(dir, ['rev-parse', 'HEAD']).trim();
      const res = checkRange({ cwd: dir, base, head, profile: 'ttl-field', counted });
      if (res.rejected.length !== 1) failures.push(`range: ${name}: expected 1 rejection, got ${res.rejected.length}`);
    }
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
  return failures;
}

function selfTest() {
  const failures = [];
  const seen = {};
  for (const [profile, accept, name, lines] of FIXTURES) {
    const reason = checkHunk(profile, lines);
    const accepted = reason === null;
    seen[`${profile}:${accept}`] = (seen[`${profile}:${accept}`] ?? 0) + 1;
    if (accepted !== accept) failures.push(`${profile}: "${name}": expected ${accept ? 'accept' : 'reject'}, got ${reason ?? 'accept'}`);
    else console.log(`ok   ${profile} ${accept ? 'accept' : 'reject'}: ${name}`);
  }
  for (const profile of Object.keys(PROFILES)) {
    for (const accept of [true, false]) {
      if (!seen[`${profile}:${accept}`]) failures.push(`${profile}: no ${accept ? 'accepted' : 'rejected'} fixture`);
    }
  }
  const range = selfTestRange();
  if (range.length === 0) console.log('ok   range: counted files skipped; mechanical range accepted; non-mechanical hunk, add, delete and rename rejected');
  failures.push(...range);
  for (const f of failures) console.log(`FAIL ${f}`);
  console.log(failures.length === 0 ? `self-test passed (${FIXTURES.length} fixtures + range)` : `self-test FAILED (${failures.length})`);
  return failures.length === 0 ? 0 : 1;
}

// ---------------------------------------------------------------------- main

function parseArgs(argv) {
  const out = { counted: [] };
  for (let i = 0; i < argv.length; i += 1) {
    const arg = argv[i];
    if (arg === '--self-test') out.selfTest = true;
    else if (arg === '--base' || arg === '--head' || arg === '--profile') out[arg.slice(2)] = argv[++i];
    else if (arg === '--counted') {
      while (i + 1 < argv.length && !argv[i + 1].startsWith('--')) out.counted.push(argv[++i]);
    } else throw new Error(`unknown argument ${arg}`);
  }
  return out;
}

function main() {
  let args;
  try {
    args = parseArgs(process.argv.slice(2));
  } catch (e) {
    console.error(e.message);
    return 2;
  }
  if (args.selfTest) return selfTest();
  if (!args.base || !args.head || !PROFILES[args.profile]) {
    console.error('usage: --base <ref> --head <ref> --profile ttl-field|g1|g2 [--counted <path>...] | --self-test');
    return 2;
  }
  let cwd;
  let res;
  try {
    cwd = git(process.cwd(), ['rev-parse', '--show-toplevel']).trim();
    res = checkRange({ cwd, base: args.base, head: args.head, profile: args.profile, counted: args.counted });
  } catch (e) {
    console.error(e.message);
    return 2;
  }
  for (const s of res.skipped) console.log(`counted (skipped): ${s}`);
  printRejected(res.rejected);
  console.log(
    `profile ${args.profile}: ${res.files} non-counted files, ${res.hunks} hunks, ${res.rejected.length} rejected`,
  );
  return res.rejected.length === 0 ? 0 : 1;
}

process.exitCode = main();
