CREATE TABLE IF NOT EXISTS cursor_state (
    id      SERIAL PRIMARY KEY,
    name    VARCHAR(64) NOT NULL UNIQUE,
    seq     BIGINT NOT NULL DEFAULT 0,
    updated TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

INSERT INTO cursor_state (name, seq) VALUES ('relayer', 0)
ON CONFLICT (name) DO NOTHING;
