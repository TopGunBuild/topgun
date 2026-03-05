-- Collaborative Tasks example schema
-- Run this against your PostgreSQL database before starting the app:
--   psql -f schema.sql

CREATE DATABASE collaborative_tasks;

\c collaborative_tasks;

CREATE TABLE IF NOT EXISTS tasks (
  id            SERIAL PRIMARY KEY,
  title         TEXT NOT NULL,
  status        TEXT NOT NULL DEFAULT 'todo',
  assignee      TEXT,
  created_at    TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- Seed data (optional)
INSERT INTO tasks (title, status, assignee) VALUES
  ('Design landing page', 'todo', 'alice'),
  ('Set up CI pipeline', 'in-progress', 'bob'),
  ('Write API docs', 'done', 'carol');
