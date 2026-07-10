# CMS Example

A headless CMS API built with [Axum](https://github.com/tokio-rs/axum) and PostgreSQL, used to demonstrate seeding a real database with [rngo](../../).

## Setup

**Start Postgres:**

```sh
docker compose up -d
```

**Configure the database URL:**

```sh
echo 'DATABASE_URL=postgres://cms:cms@localhost:5432/cms' > .env
```

**Start the API** (runs migrations automatically):

```sh
cargo run
```

The server listens on `http://localhost:3000`.

## Seeding with rngo

From the repo root, run:

```sh
cargo run -p rngo-cli -- run --dir examples/cms
```

This generates ~365 authors and ~1095 posts over the year 2024 and streams SQL `INSERT` statements directly into Postgres via `psql`. Requires `psql` to be installed locally and `DATABASE_URL` to be set in your environment (or `.env`).

To preview the generated events without writing to the database:

```sh
cargo run -p rngo-cli -- run --dir examples/cms --stdout
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
