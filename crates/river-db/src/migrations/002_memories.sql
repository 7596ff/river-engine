-- Memories table for semantic search
CREATE TABLE IF NOT EXISTS memories (
    id BLOB PRIMARY KEY,           -- 128-bit snowflake
    content TEXT NOT NULL,
    embedding BLOB NOT NULL,       -- f32 vector as bytes
    source TEXT NOT NULL,          -- 'message', 'file', 'agent'
    timestamp INTEGER NOT NULL,
    expires_at INTEGER,            -- NULL for permanent
    metadata TEXT                  -- JSON
);

CREATE INDEX IF NOT EXISTS idx_memories_source ON memories(source);
CREATE INDEX IF NOT EXISTS idx_memories_timestamp ON memories(timestamp);
CREATE INDEX IF NOT EXISTS idx_memories_expires ON memories(expires_at) WHERE expires_at IS NOT NULL;
