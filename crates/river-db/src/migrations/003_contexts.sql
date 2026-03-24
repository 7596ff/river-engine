-- Contexts table for conversation persistence
CREATE TABLE IF NOT EXISTS contexts (
    id BLOB PRIMARY KEY,              -- 128-bit snowflake (type 0x06)
    archived_at BLOB,                 -- Snowflake generated at rotation, NULL while active
    token_count INTEGER,              -- Last known prompt_tokens from API
    summary TEXT,                     -- Summary provided at rotation
    blob BLOB                         -- JSONL content, NULL while active
);
