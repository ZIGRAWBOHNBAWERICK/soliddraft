CREATE TABLE IF NOT EXISTS feedback (
  id TEXT PRIMARY KEY,
  created_at TEXT NOT NULL,
  payload TEXT NOT NULL CHECK (json_valid(payload)),
  issue_url TEXT,
  synced_at TEXT
);
CREATE INDEX IF NOT EXISTS feedback_pending ON feedback(created_at) WHERE issue_url IS NULL;
