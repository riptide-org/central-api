CREATE TABLE IF NOT EXISTS agents
(
    id BIGSERIAL PRIMARY KEY NOT NULL,
    public_id text NOT NULL,
    created_at timestamp with time zone DEFAULT (now() at time zone 'utc'),
    last_signin timestamp with time zone DEFAULT (now() at time zone 'utc'),
    secure_key text NOT NULL
);