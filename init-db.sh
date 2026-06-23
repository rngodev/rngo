#!/bin/sh
sqlite3 db.sqlite <<'SQL'
CREATE TABLE IF NOT EXISTS USERS (
    id         INTEGER PRIMARY KEY,
    email      TEXT NOT NULL,
    name       TEXT NOT NULL,
    created_at TEXT NOT NULL
);
DELETE FROM USERS;
SQL
