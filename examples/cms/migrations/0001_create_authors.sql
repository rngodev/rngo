CREATE TABLE authors (
    id BIGINT PRIMARY KEY,
    name TEXT NOT NULL,
    email TEXT NOT NULL,
    bio TEXT,
    created_at BIGINT NOT NULL
);
