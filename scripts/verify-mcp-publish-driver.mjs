#!/usr/bin/env node
// Stdio JSON-RPC driver invoked by scripts/verify-mcp-publish.sh.
// TEST_MODE=boot (default): just initialize + tools/list — no TopGun needed.
// TEST_MODE=full: also drive the 8 P15 smoke calls + 2 edge cases against $TOPGUN_URL.

import { spawn } from 'node:child_process';
import { setTimeout as delay } from 'node:timers/promises';

const TEST_MODE = process.env.TEST_MODE || 'boot';
const TOPGUN_URL = process.env.TOPGUN_URL || 'ws://localhost:8080';

const proc = spawn('node', ['node_modules/@topgunbuild/mcp-server/dist/cli.js'], {
  env: { ...process.env, TOPGUN_URL, NODE_ENV: 'production' },
  stdio: ['pipe', 'pipe', 'inherit'],
});

let nextId = 1;
const pending = new Map();
let buf = '';

proc.stdout.on('data', (chunk) => {
  buf += chunk.toString('utf8');
  let nl;
  while ((nl = buf.indexOf('\n')) >= 0) {
    const line = buf.slice(0, nl).trim();
    buf = buf.slice(nl + 1);
    if (!line) continue;
    let msg;
    try {
      msg = JSON.parse(line);
    } catch {
      continue;
    }
    if (msg.id != null && pending.has(msg.id)) {
      pending.get(msg.id)(msg);
      pending.delete(msg.id);
    }
  }
});

function rpc(method, params, timeoutMs = 8000) {
  const id = nextId++;
  return new Promise((resolve, reject) => {
    const t = setTimeout(() => {
      pending.delete(id);
      reject(new Error(`rpc timeout: ${method}`));
    }, timeoutMs);
    pending.set(id, (msg) => {
      clearTimeout(t);
      resolve(msg);
    });
    proc.stdin.write(JSON.stringify({ jsonrpc: '2.0', id, method, params }) + '\n');
  });
}
function notify(method, params) {
  proc.stdin.write(JSON.stringify({ jsonrpc: '2.0', method, params }) + '\n');
}

const results = [];
function record(name, ok, summary) {
  results.push({ name, ok, summary });
  console.log(`[${ok ? 'PASS' : 'FAIL'}] ${name} — ${summary}`);
}

function parseText(resp) {
  if (resp?.error) return { error: resp.error };
  const content = resp?.result?.content;
  if (
    Array.isArray(content) &&
    content[0]?.type === 'text' &&
    typeof content[0].text === 'string'
  ) {
    try {
      return {
        parsed: JSON.parse(content[0].text),
        text: content[0].text,
        isError: resp?.result?.isError === true,
      };
    } catch {
      return { text: content[0].text, isError: resp?.result?.isError === true };
    }
  }
  return { content, isError: resp?.result?.isError === true };
}

async function main() {
  // Boot phase — every mode runs this.
  const init = await rpc('initialize', {
    protocolVersion: '2024-11-05',
    capabilities: { tools: {} },
    clientInfo: { name: 'verify-mcp-publish', version: '1.0.0' },
  });
  if (init.error) throw new Error('initialize failed: ' + JSON.stringify(init.error));
  notify('notifications/initialized');

  const list = await rpc('tools/list', {});
  if (list.error) throw new Error('tools/list failed: ' + JSON.stringify(list.error));
  const toolNames = list.result.tools.map((t) => t.name).sort();
  const EXPECTED = [
    'topgun_explain',
    'topgun_list_maps',
    'topgun_mutate',
    'topgun_query',
    'topgun_schema',
    'topgun_search',
    'topgun_stats',
    'topgun_subscribe',
  ];
  const ok = toolNames.length === EXPECTED.length && EXPECTED.every((n) => toolNames.includes(n));
  record('boot: 8 tools registered', ok, toolNames.join(','));
  if (!ok) throw new Error('expected 8 tools, got ' + toolNames.length);

  if (TEST_MODE !== 'full') {
    proc.stdin.end();
    await delay(200);
    try {
      proc.kill();
    } catch {}
    const fails = results.filter((r) => !r.ok).length;
    console.log(`\nBOOT: ${results.length - fails}/${results.length} PASS`);
    process.exit(fails === 0 ? 0 : 1);
  }

  // Full smoke matrix — requires reachable TopGun server.
  async function call(name, args) {
    return rpc('tools/call', { name, arguments: args });
  }

  let r, p;

  r = await call('topgun_list_maps', {});
  p = parseText(r);
  record(
    'topgun_list_maps',
    !r.error && !p.isError,
    JSON.stringify(p.parsed ?? p.text).slice(0, 100),
  );

  r = await call('topgun_mutate', {
    map: 'tasks',
    operation: 'set',
    key: 'task-1',
    data: { title: 'ship HN', done: false },
  });
  p = parseText(r);
  record(
    'topgun_mutate task-1',
    !r.error && !p.isError,
    JSON.stringify(p.parsed ?? p.text).slice(0, 100),
  );
  await delay(300);

  r = await call('topgun_query', { map: 'tasks', limit: 20 });
  p = parseText(r);
  const containsTask1 = JSON.stringify(p.parsed ?? p.text).includes('ship HN');
  record(
    'topgun_query tasks',
    !r.error && !p.isError && containsTask1,
    `containsTask1=${containsTask1}`,
  );

  r = await call('topgun_schema', { map: 'tasks' });
  p = parseText(r);
  const schemaText = JSON.stringify(p.parsed ?? p.text);
  const titleOk = /title.*?(string|enum\()/.test(schemaText);
  const doneOk = /done.*?(bool|boolean)/.test(schemaText);
  record(
    'topgun_schema tasks',
    !r.error && !p.isError && titleOk && doneOk,
    `title=${titleOk} done=${doneOk}`,
  );

  let batch = 0;
  for (let i = 2; i <= 6; i++) {
    r = await call('topgun_mutate', {
      map: 'tasks',
      operation: 'set',
      key: `task-${i}`,
      data: { title: `HN polish task #${i}`, done: false },
    });
    p = parseText(r);
    if (!r.error && !p.isError) batch++;
  }
  record('topgun_mutate batch x5', batch === 5, `${batch}/5`);
  await delay(500);

  r = await call('topgun_search', { map: 'tasks', query: 'HN', limit: 20 });
  p = parseText(r);
  record(
    'topgun_search HN',
    !r.error && !p.isError,
    JSON.stringify(p.parsed ?? p.text).slice(0, 100),
  );

  r = await call('topgun_stats', {});
  p = parseText(r);
  record('topgun_stats', !r.error && !p.isError, JSON.stringify(p.parsed ?? p.text).slice(0, 100));

  r = await call('topgun_explain', { map: 'tasks', filter: { done: false } });
  p = parseText(r);
  record(
    'topgun_explain done=false',
    !r.error && !p.isError,
    JSON.stringify(p.parsed ?? p.text).slice(0, 100),
  );

  // Edge cases — must produce a graceful response, not a hang or crash.
  r = await call('topgun_query', { map: 'tasks_nonexistent_xyz_check', limit: 5 });
  p = parseText(r);
  const txt = JSON.stringify(p.parsed ?? p.text ?? r.error ?? '');
  const graceful =
    !!r.error ||
    p.isError === true ||
    /empty|not found|no (results|map|records|rows)|0 (records|rows|results)/i.test(txt);
  record('edge: nonexistent map', graceful, txt.slice(0, 100));

  r = await call('topgun_mutate', {
    map: 'tasks',
    operation: 'set',
    key: 'invalid-data',
    data: 'not-an-object',
  });
  p = parseText(r);
  const txt2 = JSON.stringify(p.parsed ?? p.text ?? r.error ?? '');
  const gracefulInvalid = !!r.error || p.isError === true || /invalid|error|expected/i.test(txt2);
  record('edge: invalid data', gracefulInvalid, txt2.slice(0, 100));

  proc.stdin.end();
  await delay(300);
  try {
    proc.kill();
  } catch {}

  const passes = results.filter((r) => r.ok).length;
  const fails = results.filter((r) => !r.ok).length;
  console.log(`\nFULL: ${passes}/${results.length} PASS, ${fails} FAIL`);
  process.exit(fails === 0 ? 0 : 1);
}

main().catch((e) => {
  console.error('FATAL:', e?.stack || e);
  try {
    proc.kill();
  } catch {}
  process.exit(2);
});
