-- Quote ledger: append-only events + idempotency for at-least-once clients.
-- All ordering guarantees are per quote_id (seq is monotonic per quote).

CREATE TABLE IF NOT EXISTS events (
    quote_id TEXT NOT NULL,
    seq INTEGER NOT NULL,
    event_type TEXT NOT NULL,
    payload BLOB NOT NULL,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    PRIMARY KEY (quote_id, seq)
);

CREATE INDEX IF NOT EXISTS events_quote_seq ON events (quote_id, seq);

CREATE TABLE IF NOT EXISTS idempotency_keys (
    quote_id TEXT NOT NULL,
    client_command_id TEXT NOT NULL,
    first_seq INTEGER NOT NULL,
    last_seq INTEGER NOT NULL,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    PRIMARY KEY (quote_id, client_command_id)
);
