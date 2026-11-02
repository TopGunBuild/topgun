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
// --base and --head are resolved to commit SHAs once, at the start, so a ref
// that moves during the run cannot make the listing and the per-file diffs
// disagree.
//
// A row of the change listing (`git diff -z --name-status -M -C`, NUL-separated
// so no path is quoted or escaped) is skipped only when every path in it is
// given to --counted (repo-relative): counted files are reviewed by hand and
// are not limited by this filter. A rename or copy with one counted side is
// checked like any other row. For every other row:
//   - the file must be modified in place: an added, deleted, renamed, copied
//     or type-changed file is rejected, and so is a file holding U+0001 or
//     U+0002, the marks the lexer frames literals with;
//   - every hunk (git's default three lines of context) is reduced to its old
//     side R and its new side A. Whitespace is removed outside string and char
//     literals and comments, so that a rustfmt reflow compares equal; inside a
//     literal or a comment every character is kept, so an edit there is never
//     mechanical. Whether a hunk side starts inside a literal or a block
//     comment is read from the whole file, not guessed from the hunk.
//
// Profile ttl-field (insertion only): walk R and A together; where they
// differ, A must begin, at that point and outside any literal or comment, with
// an insertion allowed there, which is skipped before the walk goes on: inside
// a brace field list, `,..` or `..` directly before its `}` (`..` only after
// `,` or `{`), `,ttl_ms:None` before `}` or `,`, `ttl_ms:None,` after `{` or
// `,`; anywhere, a single `,` directly before `}` or `)` (rustfmt's trailing
// comma). Any other difference, or text of R left over, rejects the hunk. So
// nothing is ever removed, and moving `..` or `ttl_ms: None` from one pattern
// or literal to another is rejected.
//
// Profiles g1 / g2: a removed `//` comment line is dropped from R only when it
// belongs to a run of removed comment lines that ends directly above removed
// code lines the profile's regexes change, taken together (the doc comment of the removed
// item). The profile's regexes are stripped from the code of R (never from
// literals or comments); g2's gated regex is applied only when R calls one of
// its trigger functions or variants in code. Both sides are normalised (a
// single-item brace group after a path separator, `::{X}` / `::{X,}`, becomes
// `::X`, and `,)` / `,}` become `)` / `}`); the results must be equal and the
// code of A must match none of the regexes.
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
    regexes: [
      /,?_?expiration_time:(i64|u64|Option<i64>|Option<u64>)/g,
      /,\*?expiration_time(?=[,)])/g,
      /expiration_time(:[A-Za-z0-9_*]+)?,/g,
    ],
    // Applied only to hunks whose R calls one of the functions or names one of
    // the variants that lose the argument, in code; the compiler rejects the
    // deletion anywhere else.
    gated: {
      trigger: /(?<![A-Za-z0-9_])(add|add_encoded|add_backup)\(|(?<![A-Za-z0-9_])(Put|Store)\{/,
      regex: /,(0|0i64|None)(?=[,)])/g,
    },
  },
};

// ------------------------------------------------------------------ lexing
//
// The normalised form of a text is its code with whitespace removed, and each
// literal or comment kept verbatim between OPEN and CLOSE marks. A lexer state
// says where a line starts: in code, in a (raw) string, or in a block comment.

const OPEN = '\u0001';
const CLOSE = '\u0002';
const CODE = Object.freeze({ mode: 'code' });
const isIdent = (c) => c !== undefined && /[A-Za-z0-9_]/.test(c);

// Lexes `text` from `state`; returns the normalised text and the state at its end.
function lex(text, state) {
  let out = '';
  let st = state;
  let i = 0;
  if (st.mode !== 'code') out += OPEN;
  while (i < text.length) {
    const c = text[i];
    if (st.mode === 'str') {
      out += c;
      if (c === '\\' && i + 1 < text.length) {
        out += text[i + 1];
        i += 2;
        continue;
      }
      i += 1;
      if (c === '"') {
        out += CLOSE;
        st = CODE;
      }
      continue;
    }
    if (st.mode === 'raw') {
      const end = '"' + '#'.repeat(st.hashes);
      if (text.startsWith(end, i)) {
        out += end + CLOSE;
        i += end.length;
        st = CODE;
      } else {
        out += c;
        i += 1;
      }
      continue;
    }
    if (st.mode === 'block') {
      if (text.startsWith('*/', i)) {
        out += '*/';
        i += 2;
        st = st.depth === 1 ? CODE : { mode: 'block', depth: st.depth - 1 };
        if (st.mode === 'code') out += CLOSE;
      } else if (text.startsWith('/*', i)) {
        out += '/*';
        i += 2;
        st = { mode: 'block', depth: st.depth + 1 };
      } else {
        out += c;
        i += 1;
      }
      continue;
    }
    // code
    if (/\s/.test(c)) {
      i += 1;
      continue;
    }
    if (text.startsWith('//', i)) {
      const nl = text.indexOf('\n', i);
      const end = nl === -1 ? text.length : nl;
      out += OPEN + text.slice(i, end) + CLOSE;
      i = end;
      continue;
    }
    if (text.startsWith('/*', i)) {
      out += OPEN + '/*';
      i += 2;
      st = { mode: 'block', depth: 1 };
      continue;
    }
    // Raw string: r"..", r#".."#, br#".."# (not a raw identifier r#name).
    const rawAt = c === 'r' ? i : c === 'b' && text[i + 1] === 'r' ? i + 1 : -1;
    if (rawAt !== -1 && !isIdent(text[i - 1])) {
      let j = rawAt + 1;
      while (text[j] === '#') j += 1;
      if (text[j] === '"') {
        out += OPEN + text.slice(i, j + 1);
        st = { mode: 'raw', hashes: j - rawAt - 1 };
        i = j + 1;
        continue;
      }
    }
    if (c === '"') {
      out += OPEN + '"';
      st = { mode: 'str' };
      i += 1;
      continue;
    }
    if (c === "'") {
      // A char literal is an escape or one code point closed by `'`; anything
      // else is a lifetime or a label.
      let end = -1;
      if (text[i + 1] === '\\') {
        end = text.indexOf("'", i + 3);
      } else if (i + 1 < text.length) {
        const cp = text.codePointAt(i + 1);
        const width = cp > 0xffff ? 2 : 1;
        if (text[i + 1 + width] === "'") end = i + 1 + width;
      }
      if (end !== -1) {
        out += OPEN + text.slice(i, end + 1) + CLOSE;
        i = end + 1;
        continue;
      }
    }
    out += c;
    i += 1;
  }
  if (st.mode !== 'code') out += CLOSE;
  return { out, state: st };
}

// Lexer state at the start of every line (index 0 = line 1) of a whole file.
function lineStartStates(text) {
  const states = [];
  let st = CODE;
  for (const line of text.split('\n')) {
    states.push(st);
    st = lex(line + '\n', st).state;
  }
  return states;
}

// Lexes consecutive lines; returns one normalised piece per line.
function lexLines(texts, state) {
  let st = state;
  return texts.map((t) => {
    const startState = st;
    const r = lex(t + '\n', st);
    st = r.state;
    return { out: r.out, startState };
  });
}

// Applies `fn` to the code pieces of a normalised text, leaving literals and comments untouched.
const mapCode = (s, fn) =>
  s
    .split(new RegExp(`(${OPEN}[^${CLOSE}]*${CLOSE}?)`))
    .map((piece) => (piece.startsWith(OPEN) ? piece : fn(piece)))
    .join('');
const codeOf = (s) => mapCode(s, (x) => x).split(new RegExp(`${OPEN}[^${CLOSE}]*${CLOSE}?`)).join(' ');

const trailingComma = (s) => s.replace(/,\)/g, ')').replace(/,\}/g, '}');
const singleItemGroup = (s) => s.replace(/::\{([^{},]*),?\}/g, '::$1');

// Whether position `j` of a normalised text may be in a brace field list:
// false when the bracket enclosing it in the hunk is `(` or `[`; true when it
// is `{` or lies above the hunk, where only the position rules decide.
function inBraceList(s, j) {
  let depth = 0;
  for (let k = j - 1; k >= 0; k -= 1) {
    const c = s[k];
    if (c === CLOSE) {
      k = s.lastIndexOf(OPEN, k);
    } else if (c === '}' || c === ')' || c === ']') {
      depth += 1;
    } else if (c === '{' || c === '(' || c === '[') {
      if (depth === 0) return c === '{';
      depth -= 1;
    }
  }
  return true;
}

// Each insertion with the position it is allowed at: inside a brace field
// list, `..` last before the closing brace, `ttl_ms: None` as a whole field.
const TTL_INSERTIONS = [
  [',..', (a, j, end) => a[end] === '}'],
  ['..', (a, j, end) => (a[j - 1] === ',' || a[j - 1] === '{') && a[end] === '}'],
  [',ttl_ms:None', (a, j, end) => a[end] === '}' || a[end] === ','],
  ['ttl_ms:None,', (a, j) => a[j - 1] === '{' || a[j - 1] === ','],
];

// Insertion-only alignment of the ttl-field profile.
function ttlAlign(r, a) {
  let i = 0;
  let j = 0;
  let inLiteral = false;
  while (i < r.length || j < a.length) {
    if (i < r.length && j < a.length && r[i] === a[j]) {
      if (r[i] === OPEN) inLiteral = true;
      else if (r[i] === CLOSE) inLiteral = false;
      i += 1;
      j += 1;
      continue;
    }
    if (j >= a.length) return 'the old side has text the new side lacks';
    if (inLiteral) return 'a literal or a comment differs';
    const ins = TTL_INSERTIONS.find(
      ([t, allowed]) => a.startsWith(t, j) && allowed(a, j, j + t.length) && inBraceList(a, j),
    );
    if (ins) {
      j += ins[0].length;
    } else if (a[j] === ',' && (a[j + 1] === '}' || a[j + 1] === ')')) {
      j += 1;
    } else {
      return 'the new side differs by more than an inserted field-list `..`, `ttl_ms: None` or trailing comma';
    }
  }
  return null;
}

// A hunk is a list of lines { kind: ' ' | '-' | '+', text }. `states` gives the
// lexer state at the first line of the old and of the new side (code when absent).
export function checkHunk(profileName, lines, states = {}) {
  const profile = PROFILES[profileName];
  if (!profile) throw new Error(`unknown profile ${profileName}`);
  const oldLines = lines.filter((l) => l.kind !== '+');
  const newLines = lines.filter((l) => l.kind !== '-');
  const oldLex = lexLines(
    oldLines.map((l) => l.text),
    states.old ?? CODE,
  );
  const a = lexLines(
    newLines.map((l) => l.text),
    states.new ?? CODE,
  )
    .map((p) => p.out)
    .join('');
  if (profile.kind === 'ttl') return ttlAlign(oldLex.map((p) => p.out).join(''), a);

  const regexes = [...profile.regexes];
  const stripAll = (s, res) => mapCode(s, (code) => res.reduce((acc, re) => acc.replace(re, ''), code));
  const oldCode = codeOf(oldLex.map((p) => p.out).join(''));
  if (profile.gated && profile.gated.trigger.test(oldCode)) regexes.push(profile.gated.regex);

  // A removed line that is a `//` comment and nothing else.
  const isCommentLine = (k) =>
    oldLines[k].kind === '-' &&
    oldLex[k].startState.mode === 'code' &&
    /^\s*\/\//.test(oldLines[k].text);
  // The run of removed code lines starting at `k`, joined, so that a match a
  // rustfmt line break splits is still seen.
  const stripsSomething = (k) => {
    let code = '';
    for (let m = k; m < oldLines.length && oldLines[m].kind === '-' && !isCommentLine(m); m += 1) {
      code += oldLex[m].out;
    }
    return code !== '' && stripAll(code, regexes) !== code;
  };
  const drop = new Set();
  for (let k = 0; k < oldLines.length; k += 1) {
    if (!isCommentLine(k)) continue;
    let end = k;
    while (end < oldLines.length && isCommentLine(end)) end += 1;
    if (stripsSomething(end)) {
      for (let m = k; m < end; m += 1) drop.add(m);
    }
    k = end - 1;
  }
  const r = stripAll(
    oldLex
      .filter((_, k) => !drop.has(k))
      .map((p) => p.out)
      .join(''),
    regexes,
  );
  const norm = (s) => mapCode(s, (code) => trailingComma(singleItemGroup(code)));
  if (norm(r) !== norm(a)) return 'old side minus the removed items differs from the new side';
  const newCode = codeOf(a);
  for (const re of regexes) {
    re.lastIndex = 0;
    if (re.test(newCode)) {
      re.lastIndex = 0;
      return `the new side still matches ${re}`;
    }
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
      const m = /^@@ -(\d+)(?:,\d+)? \+(\d+)(?:,\d+)? @@/.exec(line);
      current = { header: line, oldStart: Number(m[1]), newStart: Number(m[2]), lines: [] };
      hunks.push(current);
    } else if (current && (line.startsWith(' ') || line.startsWith('-') || line.startsWith('+'))) {
      current.lines.push({ kind: line[0], text: line.slice(1) });
    } else if (current && line.startsWith('\\')) {
      // "\ No newline at end of file" carries no text.
    }
  }
  return hunks;
}

// Parses `git diff -z --name-status` output into rows { code, paths }.
function parseNameStatusZ(out) {
  const tokens = out.split('\0');
  const rows = [];
  let k = 0;
  while (k < tokens.length && tokens[k] !== '') {
    const code = tokens[k];
    const n = /^[RC]/.test(code) ? 2 : 1;
    rows.push({ code, paths: tokens.slice(k + 1, k + 1 + n) });
    k += 1 + n;
  }
  return rows;
}

// The lexer state at line `start` (1-based) of a file whose hunk side has
// `count` lines; a file shorter than the hunk is a contradiction, not code.
export function stateAt(states, start, count) {
  const first = Math.max(start, 1) - 1;
  if (first + Math.max(count, 1) > states.length) {
    throw new Error(`a hunk at line ${start} with ${count} lines does not fit a file of ${states.length} lines`);
  }
  return states[first];
}

// Returns { rejected: [...], files, hunks, skipped }.
export function checkRange({ cwd, base, head, profile, counted }) {
  const countedSet = new Set(counted);
  const rejected = [];
  let files = 0;
  let hunkCount = 0;
  const skipped = [];
  if ([base, head].some((ref) => ref.startsWith('-'))) throw new Error('--base and --head must not start with -');
  const shas = git(cwd, ['rev-parse', `${base}^{commit}`, `${head}^{commit}`]).trim().split('\n');
  if (shas.length !== 2 || !shas.every((sha) => /^[0-9a-f]{40,64}$/.test(sha))) {
    throw new Error(`--base and --head did not resolve to two commits: ${shas.join(' ')}`);
  }
  const [baseSha, headSha] = shas;
  const status = git(cwd, ['diff', '--no-ext-diff', '-z', '--name-status', '-M', '-C', baseSha, headSha, '--']);
  for (const { code, paths } of parseNameStatusZ(status)) {
    if (paths.every((p) => countedSet.has(p))) {
      skipped.push(`${code}\t${paths.join(' -> ')}`);
      continue;
    }
    if (code !== 'M') {
      rejected.push({ file: paths.join(' -> '), reason: `status ${code}: only in-place modification is allowed outside counted files` });
      continue;
    }
    files += 1;
    const path = paths[0];
    const diff = git(cwd, ['diff', '--no-ext-diff', '--no-color', '-U3', baseSha, headSha, '--', path]);
    if (/^Binary files /m.test(diff)) {
      rejected.push({ file: path, reason: 'binary change' });
      continue;
    }
    const hunks = parseHunks(diff);
    if (hunks.length === 0) {
      rejected.push({ file: path, reason: 'listed as modified but its diff has no hunk' });
      continue;
    }
    const oldText = git(cwd, ['show', `${baseSha}:${path}`]);
    const newText = git(cwd, ['show', `${headSha}:${path}`]);
    // The lexer frames literals and comments with these two characters.
    if (/[\u0001\u0002]/.test(oldText + newText)) {
      rejected.push({ file: path, reason: 'the file contains U+0001 or U+0002, which the filter cannot check' });
      continue;
    }
    const oldStates = lineStartStates(oldText);
    const newStates = lineStartStates(newText);
    for (const hunk of hunks) {
      hunkCount += 1;
      const states = {
        old: stateAt(oldStates, hunk.oldStart, hunk.lines.filter((l) => l.kind !== '+').length),
        new: stateAt(newStates, hunk.newStart, hunk.lines.filter((l) => l.kind !== '-').length),
      };
      const reason = checkHunk(profile, hunk.lines, states);
      if (reason) rejected.push({ file: path, header: hunk.header, reason, lines: hunk.lines });
    }
  }
  return { rejected, files, hunks: hunkCount, skipped, base: baseSha, head: headSha };
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
  ['ttl-field', true, 'one-line pattern reflowed by rustfmt to the multi-line form with ..', hunk(`
         match stored {
-            RecordValue::Lww { value, timestamp } => Some((value.clone(), timestamp.clone())),
+            RecordValue::Lww {
+                value, timestamp, ..
+            } => Some((value.clone(), timestamp.clone())),
             _ => None,
`)],
  ['ttl-field', true, 'literals with a quote char, a lifetime and a raw string gain a field', hunk(`
-    let v = Foo::<'a> { q: '"', s: r#"a "b" c"#, timestamp: ts };
+    let v = Foo::<'a> { q: '"', s: r#"a "b" c"#, timestamp: ts, ttl_ms: None };
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
  ['ttl-field', false, 'whitespace removed inside a string literal', hunk(`
-    println!("a b");
+    println!("ab");
`)],
  ['ttl-field', false, 'comment re-flowed', hunk(`
-    /* the store keeps the record
-       until it is read */
+    /* the store keeps
+       the record until it is read */
     let a = 1;
`)],
  ['ttl-field', false, 'whitespace removed in a string after a char literal holding a quote', hunk(`
-    let q = '"'; let s = "a b";
+    let q = '"'; let s = "ab";
`)],
  ['ttl-field', false, 'whitespace removed inside a raw string holding quotes', hunk(`
-    let s = r#"x" y"#;
+    let s = r#"x"y"#;
`)],
  ['ttl-field', false, '.. moved from one pattern to another', hunk(`
-        Foo { a, .. } => f(a),
-        Bar { b } => g(b),
+        Foo { a } => f(a),
+        Bar { b, .. } => g(b),
`)],
  ['ttl-field', false, 'ttl_ms: None moved from one literal to another', hunk(`
-    let a = Foo { x: 1, ttl_ms: None };
-    let b = Bar { y: 2 };
+    let a = Foo { x: 1 };
+    let b = Bar { y: 2, ttl_ms: None };
`)],
  ['ttl-field', false, '.. inserted inside a string literal', hunk(`
-    let s = "ab";
+    let s = "a..b";
`)],
  ['ttl-field', false, '.. inserted into an index, making a range', hunk(`
-    g(&v[1]);
+    g(&v[1..]);
`)],
  ['ttl-field', false, '.. inserted before an index, making a range', hunk(`
-    let s = &x[0];
+    let s = &x[..0];
`)],
  ['ttl-field', false, '.. inserted before a field, making a struct update', hunk(`
-    let a = S { x, y };
+    let a = S { x, ..y };
`)],
  ['ttl-field', false, '.. inserted into a range', hunk(`
-    for i in 0..n {}
+    for i in 0....n {}
`)],
  ['ttl-field', false, '.. inserted after a field value, making a range', hunk(`
-    let a = S { x: 0 };
+    let a = S { x: 0.. };
`)],
  ['ttl-field', false, 'ttl_ms: None inserted into a call', hunk(`
-    f(x);
+    f(x, ttl_ms: None);
`)],
  ['ttl-field', false, 'comma inserted before ]', hunk(`
-    let v = [a];
+    let v = [a,];
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
  ['g1', true, 'doc comment above a removed field that rustfmt split over two lines', hunk(`
     let c = StorageConfig {
         a: 1,
-        /// Records expire after this long by default.
-        default_ttl_millis:
-            5000,
         b: 2,
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
  ['g1', false, 'unrelated comment line deleted with no removed item below it', hunk(`
     let p = 1;
-    // see docs/expiry.md - the old default was buggy
     let q = 2;
`)],
  ['g1', false, 'removed type name deleted from a string literal', hunk(`
-    log("ExpiryPolicy::NONE, used");
+    log("used");
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
  ['g2', false, 'trigger only in a comment does not arm the gated regex', hunk(`
     // add(0) was here
-    let x = compute(a, 0);
+    let x = compute(a);
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

// A string literal long enough that a hunk in its middle has no line outside it.
const LONG_STRING = (middle) =>
  `fn s() {\n    let t = "\nl1\nl2\nl3\nl4\n${middle}\nl6\nl7\nl8\nl9\n";\n}\n`;

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
    write('kept_counted.rs', 'fn k() {\n    let a = 1;\n    let b = 2;\n    let c = 3;\n}\n');
    write('gone.rs', 'fn e() {}\n');
    write('moved.rs', 'fn f() {\n    let a = 1;\n    let b = 2;\n    let c = 3;\n}\n');
    write('café.rs', 'fn g() {\n    let n = 1;\n}\n');
    write('long.rs', LONG_STRING('a b'));
    write('marks.rs', 'fn m() {\n    let s = "a\u0002}";\n}\n');
    write('ünï.rs', 'fn u() {\n    let v = RecordValue::Lww { value: x, timestamp: t };\n}\n');
    runGit(dir, ['add', '-A']);
    runGit(dir, ['commit', '-q', '-m', 'base']);
    const base = runGit(dir, ['rev-parse', 'HEAD']).trim();

    // Mechanical change in a.rs and in a non-ASCII path; counted file rewritten; new counted file.
    write('a.rs', 'fn a() {\n    let v = RecordValue::Lww { value: x, timestamp: t, ttl_ms: None };\n}\n');
    write('ünï.rs', 'fn u() {\n    let v = RecordValue::Lww { value: x, timestamp: t, ttl_ms: None };\n}\n');
    write('counted.rs', 'fn d() { anything_goes(); }\n');
    write('new_counted.rs', 'fn g() {}\n');
    runGit(dir, ['add', '-A']);
    runGit(dir, ['commit', '-q', '-m', 'ok']);
    const ok = runGit(dir, ['rev-parse', 'HEAD']).trim();
    const counted = ['counted.rs', 'new_counted.rs', 'kept_counted.rs'];
    const good = checkRange({ cwd: dir, base, head: ok, profile: 'ttl-field', counted });
    if (good.rejected.length !== 0) failures.push(`range: a mechanical range was rejected: ${JSON.stringify(good.rejected)}`);
    if (good.skipped.length !== 2) failures.push(`range: expected 2 counted files skipped, got ${good.skipped.length}`);

    // Each forbidden kind of change, one per commit on top of `ok`.
    const forbidden = [
      ['non-mechanical hunk', () => write('b.rs', 'fn b() {\n    let n = 2;\n}\n')],
      ['added file', () => write('added.rs', 'fn h() {}\n')],
      ['deleted file', () => rmSync(join(dir, 'gone.rs'))],
      ['renamed file', () => renameSync(join(dir, 'moved.rs'), join(dir, 'renamed.rs'))],
      ['non-mechanical hunk in a non-ASCII path', () => write('café.rs', 'fn g() {\n    let n = 2;\n}\n')],
      ['counted file renamed to a non-counted path', () => renameSync(join(dir, 'kept_counted.rs'), join(dir, 'evil.rs'))],
      ['whitespace removed in a hunk wholly inside a string literal', () => write('long.rs', LONG_STRING('ab'))],
      ['a file holding the lexer marks', () => write('marks.rs', 'fn m() {\n    let s = "a\u0002,}";\n}\n')],
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
  try {
    stateAt([CODE, CODE], 2, 3);
    failures.push('stateAt: a hunk longer than its file was accepted');
  } catch {
    console.log('ok   stateAt: a hunk longer than its file is an error');
  }
  if (stateAt([CODE, { mode: 'str' }, CODE], 2, 2).mode !== 'str') failures.push('stateAt: wrong line');
  const range = selfTestRange();
  if (range.length === 0) {
    console.log(
      'ok   range: counted files skipped; mechanical range (non-ASCII path included) accepted; non-mechanical hunk, add, delete, rename, ' +
        'non-ASCII path, counted-to-non-counted rename, an edit inside a multi-line string and a file holding the lexer marks rejected',
    );
  }
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
  console.log(`base ${res.base}, head ${res.head}`);
  for (const s of res.skipped) console.log(`counted (skipped): ${s}`);
  printRejected(res.rejected);
  console.log(
    `profile ${args.profile}: ${res.files} non-counted files, ${res.hunks} hunks, ${res.rejected.length} rejected`,
  );
  return res.rejected.length === 0 ? 0 : 1;
}

process.exitCode = main();
