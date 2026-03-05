/**
 * REST CRUD routes for tasks.
 *
 * Uses pg (node-postgres) directly -- no ORM, no TopGun for persistence.
 * After each mutation the route calls topgun-setup helpers to broadcast
 * the change to all connected real-time clients.
 */

import { Router } from 'express';
import type { Pool } from 'pg';
import { publishTaskUpdate, publishTaskDelete } from '../topgun-setup.js';

export function createTaskRoutes(pool: Pool): Router {
  const router = Router();

  // GET /api/tasks -- list all tasks
  router.get('/', async (_req, res) => {
    try {
      const result = await pool.query(
        'SELECT id, title, status, assignee, created_at FROM tasks ORDER BY created_at DESC'
      );
      res.json(result.rows);
    } catch (err) {
      console.error('[tasks] GET error:', err);
      res.status(500).json({ error: 'Failed to fetch tasks' });
    }
  });

  // POST /api/tasks -- create a new task
  router.post('/', async (req, res) => {
    const { title, status = 'todo', assignee = null } = req.body;
    if (!title) {
      return res.status(400).json({ error: 'title is required' });
    }
    try {
      const result = await pool.query(
        'INSERT INTO tasks (title, status, assignee) VALUES ($1, $2, $3) RETURNING *',
        [title, status, assignee]
      );
      const task = result.rows[0];
      publishTaskUpdate(task);
      res.status(201).json(task);
    } catch (err) {
      console.error('[tasks] POST error:', err);
      res.status(500).json({ error: 'Failed to create task' });
    }
  });

  // PUT /api/tasks/:id -- update a task
  router.put('/:id', async (req, res) => {
    const { id } = req.params;
    const { title, status, assignee } = req.body;
    try {
      const result = await pool.query(
        `UPDATE tasks
         SET title    = COALESCE($1, title),
             status   = COALESCE($2, status),
             assignee = COALESCE($3, assignee)
         WHERE id = $4
         RETURNING *`,
        [title, status, assignee, id]
      );
      if (result.rows.length === 0) {
        return res.status(404).json({ error: 'Task not found' });
      }
      const task = result.rows[0];
      publishTaskUpdate(task);
      res.json(task);
    } catch (err) {
      console.error('[tasks] PUT error:', err);
      res.status(500).json({ error: 'Failed to update task' });
    }
  });

  // DELETE /api/tasks/:id -- delete a task
  router.delete('/:id', async (req, res) => {
    const { id } = req.params;
    try {
      const result = await pool.query(
        'DELETE FROM tasks WHERE id = $1 RETURNING id',
        [id]
      );
      if (result.rows.length === 0) {
        return res.status(404).json({ error: 'Task not found' });
      }
      publishTaskDelete(Number(id));
      res.json({ deleted: true });
    } catch (err) {
      console.error('[tasks] DELETE error:', err);
      res.status(500).json({ error: 'Failed to delete task' });
    }
  });

  return router;
}
