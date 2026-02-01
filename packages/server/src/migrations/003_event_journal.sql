-- Event Journal table
-- Event Journal / Ringbuffer
--
-- This table stores all map changes as an append-only log for:
-- - Audit trails (regulatory compliance)
-- - CDC (Change Data Capture) for external systems
-- - Event replay for debugging/recovery
-- - Time-travel queries

CREATE TABLE IF NOT EXISTS event_journal (
  sequence BIGINT PRIMARY KEY,
  type VARCHAR(10) NOT NULL CHECK (type IN ('PUT', 'UPDATE', 'DELETE')),
  map_name VARCHAR(255) NOT NULL,
  key VARCHAR(1024) NOT NULL,
  value JSONB,
  previous_value JSONB,
  timestamp JSONB NOT NULL,
  node_id VARCHAR(64) NOT NULL,
  metadata JSONB,
  created_at TIMESTAMPTZ DEFAULT NOW()
);

-- Indexes for common queries
CREATE INDEX IF NOT EXISTS idx_journal_map_name ON event_journal(map_name);
CREATE INDEX IF NOT EXISTS idx_journal_key ON event_journal(map_name, key);
CREATE INDEX IF NOT EXISTS idx_journal_created_at ON event_journal(created_at);
CREATE INDEX IF NOT EXISTS idx_journal_node_id ON event_journal(node_id);
CREATE INDEX IF NOT EXISTS idx_journal_type ON event_journal(type);

-- Composite index for time-range queries on specific maps
CREATE INDEX IF NOT EXISTS idx_journal_map_time ON event_journal(map_name, created_at);

-- Retention policy function
-- Run periodically to cleanup old events
CREATE OR REPLACE FUNCTION cleanup_old_journal_events(retention_days INT DEFAULT 30)
RETURNS INTEGER AS $$
DECLARE
  deleted_count INTEGER;
BEGIN
  DELETE FROM event_journal
  WHERE created_at < NOW() - (retention_days || ' days')::INTERVAL;

  GET DIAGNOSTICS deleted_count = ROW_COUNT;
  RETURN deleted_count;
END;
$$ LANGUAGE plpgsql;

-- Optional: Partitioning by time for large deployments
-- Uncomment and modify for your use case
--
-- CREATE TABLE event_journal_partitioned (
--   sequence BIGINT NOT NULL,
--   type VARCHAR(10) NOT NULL CHECK (type IN ('PUT', 'UPDATE', 'DELETE')),
--   map_name VARCHAR(255) NOT NULL,
--   key VARCHAR(1024) NOT NULL,
--   value JSONB,
--   previous_value JSONB,
--   timestamp JSONB NOT NULL,
--   node_id VARCHAR(64) NOT NULL,
--   metadata JSONB,
--   created_at TIMESTAMPTZ DEFAULT NOW(),
--   PRIMARY KEY (sequence, created_at)
-- ) PARTITION BY RANGE (created_at);
--
-- CREATE TABLE event_journal_y2024m01 PARTITION OF event_journal_partitioned
--   FOR VALUES FROM ('2024-01-01') TO ('2024-02-01');
