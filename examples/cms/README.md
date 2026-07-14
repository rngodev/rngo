# CMS Example

A headless CMS API built with [Axum](https://github.com/tokio-rs/axum) and PostgreSQL, used to demonstrate seeding a real database with [rngo](../../).

## Setup

**Start Postgres and API:**

```sh
just start
```

The server listens on `http://localhost:3000`. This will also run migrations.

## Seeding with rngo

From the repo root, run:

```sh
just rngo-run
```

This generates ~365 authors and ~1095 posts over the year 2024 and streams SQL `INSERT` statements directly into Postgres via `psql`. Requires `psql` to be installed locally and `DATABASE_URL` to be set in your environment (or `.env`).

It will also generate a handful of API requests against the `GET /posts/slug/:slug` endpoint.

Reusable custom schemas defined under `.rngo/schemas/` are referenced from the effects by name instead of being inlined:

- `name` and `email` are enumerations of realistic constant values.
- `lorem.word`, `lorem.sentence`, and `lorem.paragraph` compose into placeholder text and are used for author bios and post titles/bodies.

To truncate the Postgres database, run:

```sh
just db-truncate
```

To connect to the Postgres database, run:

```sh
just psql
```

## API

### Authors

```
GET /authors
GET /authors/:id
```

### Posts

```
GET /posts
GET /posts?status=published
GET /posts/:id
GET /posts/slug/:slug
```

### Examples

```sh
curl http://localhost:3000/authors
curl http://localhost:3000/posts?status=published
curl http://localhost:3000/posts/slug/abc-defg-hij
```
