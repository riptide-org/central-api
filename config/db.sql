CREATE TABLE IF NOT EXISTS agents
(
    id BIGSERIAL PRIMARY KEY NOT NULL,
    created_at timestamp with time zone DEFAULT (now() at time zone 'utc'),
    last_signin timestamp with time zone DEFAULT (now() at time zone 'utc'),
    unique_id text --Generated from the hardware the server agent is running on.
);