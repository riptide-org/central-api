CREATE TABLE IF NOT EXISTS agents
(
    id SERIAL PRIMARY KEY NOT NULL,
    created_at timestamp with time zone DEFAULT (now() at time zone 'utc'),
);