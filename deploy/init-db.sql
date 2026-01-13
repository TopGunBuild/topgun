-- Initialize TopGun database

-- Create extensions
CREATE EXTENSION IF NOT EXISTS "uuid-ossp";

-- System table for configuration
CREATE TABLE IF NOT EXISTS _system (
    key TEXT PRIMARY KEY,
    value JSONB,
    created_at TIMESTAMPTZ DEFAULT NOW(),
    updated_at TIMESTAMPTZ DEFAULT NOW()
);

-- Insert schema version
INSERT INTO _system (key, value) VALUES ('schema_version', '"1"')
ON CONFLICT (key) DO NOTHING;
