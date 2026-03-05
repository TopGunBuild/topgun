/**
 * Collaborative Tasks -- Express server entry point.
 *
 * Serves static files (the frontend) and mounts REST CRUD routes for tasks.
 * After each REST mutation the TopGun bridge publishes the change so browser
 * clients connected via WebSocket see updates in real time.
 */

import 'dotenv/config';
import express from 'express';
import { Pool } from 'pg';
import path from 'path';
import { fileURLToPath } from 'url';
import { createTaskRoutes } from './routes/tasks.js';
import { initTopGun, closeTopGun } from './topgun-setup.js';

const __dirname = path.dirname(fileURLToPath(import.meta.url));

const PORT = Number(process.env.PORT) || 3000;
const DATABASE_URL =
  process.env.DATABASE_URL ||
  'postgres://postgres:postgres@localhost:5432/collaborative_tasks';

// --- Postgres connection pool ---
const pool = new Pool({ connectionString: DATABASE_URL });

// --- Express app ---
const app = express();
app.use(express.json());

// Serve static frontend files (index.html, app.bundle.js, etc.)
app.use(express.static(path.join(__dirname, 'public')));

// Mount REST routes
app.use('/api/tasks', createTaskRoutes(pool));

// --- Start ---
const server = app.listen(PORT, () => {
  console.log(`[server] Listening on http://localhost:${PORT}`);

  // Initialise TopGun client (connects to the Rust server)
  initTopGun();
});

// --- Graceful shutdown ---
process.on('SIGINT', async () => {
  console.log('\n[server] Shutting down...');
  await closeTopGun();
  await pool.end();
  server.close();
  process.exit(0);
});
