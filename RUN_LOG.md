# The run log

Each `rngo eval` writes its run to `log.sqlite` in its run directory (`.rngo/evals/<id>/`, linked from `.rngo/evals/last`). `SqliteRunLog` writes the same file from Rust. SQL signals query it, and so can any SQLite client, so its tables are a public contract: a change to them bumps the run log version and gets a note in [MIGRATING.md](MIGRATING.md#run-log).

## Version

This layout is **version 1**. A log records its version in SQLite's `user_version` header (`PRAGMA user_version`), and `SqliteRunLog::VERSION` is the version a given rngo reads and writes. Opening an existing log of another version, or one written before run logs were versioned, is an error that leaves the file untouched.

All timestamps are milliseconds since the Unix epoch, and JSON columns hold JSON text (query them with SQLite's `->>` operator or `json_extract`).

## `inputs`

One row per input an effect produced and the proxy sent, in the order they were sent.

| Column | Type | |
|---|---|---|
| `id` | integer | The input's id, unique and increasing across the run. |
| `effect` | text | The key of the effect that produced it. |
| `timestamp` | integer | The input's simulated time. |
| `data` | JSON | The generated value. |
| `metadata` | JSON | An array of generator metadata: objects with a `type`, an optional `attribute` (a JSON pointer into `data`) and optional `data`. |

## `outputs`

One row per output a channel reported, such as a line a `stream` command printed.

| Column | Type | |
|---|---|---|
| `channel` | text | The key of the channel that reported it. |
| `input_id` | integer or null | The input it answers, or null for output reported outside a send (such as a `stream` command's later lines). |
| `timestamp` | integer | When it was reported (wall clock). |
| `level` | text | `info`, `warning` or `error`. |
| `data` | text | The output's text. |
| `metadata` | JSON | An array of generator metadata, as on `inputs`. |

## `metadata`

Facts about the run that aren't inputs or outputs, told apart by `type`.

| Column | Type | |
|---|---|---|
| `type` | text | What the row records; see below. |
| `segment` | text or null | A sub-key some types use. |
| `timestamp` | integer or null | When it happened, for types that have a time. |
| `input_id` | integer or null | The input it concerns, if any. |
| `output_id` | integer or null | The output (its `rowid`) it concerns, if any. |
| `data` | JSON or null | The type's own fields. |

These types are part of the contract:

- `skipped`: an attempt an effect skipped (for example, a reference with nothing to refer to). `timestamp` is the attempt's simulated time; `data` is `{ "effect": <key>, "metadata": [<generator metadata>] }`.
- `timing`: wall-clock marks for the run. `data` is `{ "key": "simulation_start" | "simulation_end", "timestamp": <ms> }`. A run that sends no input has neither.
- `signal`: one per signal, written by the audit. `data` is `{ "key": <signal key>, "status": "success", "value": <query result>, "eval": { "expectation": <expect>, "passed": <bool> } | null }`, or `{ "key": <signal key>, "status": "error", "error": <message> }`.

Types starting with `_` (such as `_unique_reference`) are rngo's own bookkeeping and may change in any release.
