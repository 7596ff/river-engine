-- Messages table
CREATE TABLE IF NOT EXISTS messages (
    id BLOB PRIMARY KEY,           -- 128-bit snowflake
    session_id TEXT NOT NULL,
    role TEXT NOT NULL,            -- 'system', 'user', 'assistant', 'tool'
    content TEXT,
    tool_calls TEXT,               -- JSON array of tool calls
    tool_call_id TEXT,             -- For tool response messages
    name TEXT,                     -- Tool name for tool responses
    created_at INTEGER NOT NULL,
    metadata TEXT                  -- JSON
);

CREATE INDEX IF NOT EXISTS idx_messages_session ON messages(session_id);
CREATE INDEX IF NOT EXISTS idx_messages_created ON messages(created_at);

-- Sessions table
CREATE TABLE IF NOT EXISTS sessions (
    id TEXT PRIMARY KEY,
    agent_name TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    last_active INTEGER NOT NULL,
    context_tokens INTEGER DEFAULT 0,
    metadata TEXT                  -- JSON
);
