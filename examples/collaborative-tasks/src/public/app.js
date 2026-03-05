/**
 * Collaborative Tasks -- browser client.
 *
 * Fetches tasks via REST on load, then subscribes to the TopGun server
 * via WebSocket for real-time updates (task changes + presence).
 * esbuild bundles this file into app.bundle.js (IIFE, browser platform).
 */

import { TopGunClient } from '@topgunbuild/client';
import { IDBAdapter } from '@topgunbuild/client';

// --- Configuration ---
const TOPGUN_WS = 'ws://localhost:8080';
const API_BASE = '/api/tasks';
const USER_ID = 'user-' + Math.random().toString(36).slice(2, 6);
const COLORS = ['#e03131', '#2f9e44', '#1971c2', '#f08c00', '#9c36b5'];

// --- State ---
let tasks = [];

// --- TopGun client (real-time layer) ---
const client = new TopGunClient({
  serverUrl: TOPGUN_WS,
  storage: new IDBAdapter(),
});

const presenceMap = client.getMap('task-presence');
const taskTopic = client.topic('task-updates');

// --- DOM helpers ---
function getColumnId(status) {
  if (status === 'in-progress') return 'col-in-progress';
  if (status === 'done') return 'col-done';
  return 'col-todo';
}

function renderTasks() {
  for (const id of ['col-todo', 'col-in-progress', 'col-done']) {
    const col = document.getElementById(id);
    const heading = col.querySelector('h2');
    col.innerHTML = '';
    col.appendChild(heading);
  }
  for (const task of tasks) {
    const card = document.createElement('div');
    card.className = 'card';
    card.dataset.id = task.id;
    card.innerHTML = `
      <strong>${escapeHtml(task.title)}</strong>
      <div class="assignee">${task.assignee ? escapeHtml(task.assignee) : 'Unassigned'}</div>
      <div>${statusButtons(task)}</div>
    `;
    document.getElementById(getColumnId(task.status)).appendChild(card);
  }
}

function statusButtons(task) {
  const statuses = ['todo', 'in-progress', 'done'];
  return statuses
    .filter((s) => s !== task.status)
    .map((s) => `<button class="status-btn" onclick="moveTask(${task.id},'${s}')">${s}</button>`)
    .join(' ');
}

function escapeHtml(str) {
  const d = document.createElement('div');
  d.textContent = str;
  return d.innerHTML;
}

// --- REST API calls ---
async function loadTasks() {
  const res = await fetch(API_BASE);
  tasks = await res.json();
  renderTasks();
}

window.moveTask = async function moveTask(id, newStatus) {
  await fetch(`${API_BASE}/${id}`, {
    method: 'PUT',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ status: newStatus }),
  });
  // Optimistic local update; the TopGun subscription also fires
  const t = tasks.find((x) => x.id === id);
  if (t) {
    t.status = newStatus;
    renderTasks();
  }
};

// --- Add task form ---
document.getElementById('add-form').addEventListener('submit', async (e) => {
  e.preventDefault();
  const input = document.getElementById('new-title');
  const title = input.value.trim();
  if (!title) return;
  const res = await fetch(API_BASE, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ title }),
  });
  const task = await res.json();
  tasks.unshift(task);
  renderTasks();
  input.value = '';
});

// --- TopGun real-time subscriptions ---

// Subscribe to task updates from other clients
taskTopic.subscribe((data) => {
  if (data.action === 'update') {
    const idx = tasks.findIndex((t) => t.id === data.task.id);
    if (idx >= 0) {
      tasks[idx] = data.task;
    } else {
      tasks.unshift(data.task);
    }
    renderTasks();
  } else if (data.action === 'delete') {
    tasks = tasks.filter((t) => t.id !== data.taskId);
    renderTasks();
  }
});

// Publish own presence
presenceMap.set(USER_ID, { taskId: null, since: Date.now() });

// Render presence indicators
function renderPresence() {
  const bar = document.getElementById('presence');
  const entries = [...presenceMap.entries()];
  if (entries.length === 0) {
    bar.textContent = `Online as ${USER_ID}`;
    return;
  }
  const dots = entries
    .map(([uid], i) => {
      const color = COLORS[i % COLORS.length];
      const isSelf = uid === USER_ID ? ' (you)' : '';
      return `<span class="presence-dot" style="background:${color}"></span>${escapeHtml(uid)}${isSelf}`;
    })
    .join('&nbsp;&nbsp;');
  bar.innerHTML = `Online: ${dots}`;
}

// Poll presence map periodically (LWWMap does not emit change events)
setInterval(renderPresence, 2000);

// --- Initial load ---
loadTasks();
renderPresence();
