CREATE TABLE IF NOT EXISTS moves (
    id BLOB PRIMARY KEY,
    channel TEXT NOT NULL,
    turn_number INTEGER NOT NULL,
    summary TEXT NOT NULL,
    tool_calls TEXT,
    created_at INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_moves_channel_turn
    ON moves (channel, turn_number);
