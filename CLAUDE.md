# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Coding

Give interfaces (traits, trait methods, public structs, enums, and functions) terse `///` doc comments: one line on what it does, plus what each non-obvious argument means. Skip ones that just restate the name or signature.

Avoid inline comments. If additional context is needed for a section of code, add it in CLAUDE.md.

Always run `just fmt` and `just clippy` after making code changes. If clippy reports warnings or errors, fix them directly in the code.

## Commands

```bash
cargo test --workspace          # run all tests
cargo test -p rngo               # run library crate tests only
cargo test <test_name>          # run a single test by name
just fmt                        # format all Rust code (preferred)
cargo fmt                       # format code (rustfmt.toml: imports_granularity = "Module")
cargo fmt --check               # check formatting (used in CI)
just clippy                     # lint, matching CI (warnings are errors)
just clippy-fix                 # lint and auto-fix clippy suggestions
cargo clippy --workspace --all-targets -- -D warnings  # lint (warnings are errors in CI)
cargo build                     # build
cargo run -p rngo-cli -- run            # run simulation (writes to .rngo/runs/<UUID>/, symlinked from .rngo/runs/last)
cargo run -p rngo-cli -- run --stdout   # run simulation, print all events to stdout as JSON
just bench                      # run all criterion benchmarks (crates/rngo/benches/)
just bench sqlite_log/random    # run benchmarks whose name matches a filter
just bench --save-baseline main # save a named baseline; compare later with --baseline main
```

## Architecture

The workspace has two crates:
- `crates/rngo` (`rngo`) — core simulation library, published as the public library crate
- `crates/cli` (`rngo-cli`) — CLI binary that wires the library to the filesystem and subprocesses

### Data flow

1. **Spec** (`spec.rs`): A YAML/JSON document loaded from `.rngo/spec.yml` merged with per-file `effects/*.yml`, `channels/*.yml`, `schemas/*.yml`, and `signals/*.yml` (each merge step lives in `cli/src/run.rs::load_spec`, keyed by file stem). Defines `seed`, `start`, `end`, named `effects`, named `channels`, named custom `schemas`, and named `signals`.

2. **Dialect** (`rngo/src/parse/dialect.rs`): Converts a `spec::Spec` into builders by dispatching each effect's schema to a matching `SchemaParser`, each channel's format to a `FormatParser`, each channel's target to a `ChannelTargetParser`, and each signal to a `SignalParser`. `Dialect::primitive()` registers all built-in parsers. Three entry points, one per builder:
   - `parse_simulation` → `SimulationBuilder`
   - `parse_proxy` → `ProxyBuilder` (channel dispatch)
   - `parse_audit` → `AuditBuilder` (signal evaluation)

3. **Simulation** (`effect/simulation.rs`): An `Iterator<Item = Input>`. Each call to `next()` sorts all `Effect`s by their next timestamp offset, advances the earliest one, and pushes the resulting `Input` (or `SkippedInput` metadata) to a `RunLogWriter`, looping internally past skipped attempts until it finds a real one (or runs out). It also records `timing` metadata rows whose `data` is `{ "key", "timestamp" }` (wall-clock RFC 3339): `simulation_start` on the first `next()`, and `simulation_end` when exhausted, on `finish()`, or on drop, so signals can read both. Orchestrates many `Effect`s but doesn't share a trait with it — `Effect` models a single logical kind of input; `Simulation` merges many of them into one time-ordered run.

4. **Effect** (`effect.rs`): Also an iterator, yielding `Result<Input, SkippedInput>`. Driven by a `Trigger` (either a `Clock` for time-based firing or another `Effect` for dependency-based firing) and a `Schema` for generating values. `Input` (`{ id, effect, offset, timestamp, data, metadata }`) is the event an effect produces each time it fires.

5. **Proxy** (`rngo/src/proxy.rs`, with `channel`, `format`, and `output` submodules under `rngo/src/proxy/`): Channel dispatch. For each `Input`, looks up its effect's assigned channel, formats it via the channel's optional `Format`, and hands it to the channel's `ChannelTarget`, pushing any resulting `Output`s to the `RunLogWriter`.

6. **Audit** (`rngo/src/audit.rs`): Runs after the simulation finishes. Evaluates every named `Signal` against the completed run's log, writes each `SignalOutcome` back as metadata, and produces an `AuditReport` (pass/fail/error counts, `passed()`) that the CLI uses for its exit status.

### CLI run loop (`cli/src/run.rs`)

- `load_spec` merges `.rngo/spec.yml` with `.rngo/effects/*.yml`, `channels/*.yml`, `schemas/*.yml`, `signals/*.yml`, or `load_spec_file` loads a single file when `--spec` is passed.
- `Dialect::primitive()` parses the spec three ways (simulation, proxy, audit builders).
- `--dry-run`: only builds the `Simulation` (to validate the spec) and returns, without creating a run directory or touching channels.
- Otherwise: creates a run directory at `.rngo/runs/<UUIDv7>/`, symlinks `.rngo/runs/last` to it, writes a `spec.json` snapshot, and opens a `SqliteRunLog` (backed by `log.sqlite`) as both the simulation's `RunLogReader`/`RunLogWriter` and the proxy's writer — wrapped in `StatusWriter` (`cli/src/run/status.rs`), which renders a live effect/output counter to stderr as it forwards writes through. `StatusWriter::finish()` is called right after `proxy.finish()` so the final redraw happens before the audit prints to stdout; its in-place redraw clears the last N lines, so redrawing after other output would erase that output.
- Drives the `Simulation` iterator, sending each `Input` through the `Proxy`, then calls `proxy.finish()`, then builds and runs the `Audit` against the same `SqliteRunLog` and prints per-signal outcomes.
- `--stdout`: builds the `Proxy` with `stdout(true)`, which swaps every channel's target for a `Stdout` target (prints each input's formatted data to stdout) instead of running the real channel targets.
- `--limit N` caps the total number of effect attempts (successful + skipped) the simulation will produce.

### Channel targets (`rngo/src/proxy/channel/target/`)

`ChannelTarget` implementations, wired up by `Proxy`:
- `stream`: spawns one long-lived subprocess per channel, writes formatted event lines to its stdin.
- `exec`: runs a fresh `sh -c <command>` per event; the command string is a Handlebars template rendered with the event's JSON value.
- `stdout`: prints formatted event data to stdout; used in place of the real target when `--stdout` is passed.

An effect opts into a channel by setting `channel: <channel-key>`. The format used is resolved by merging the effect-level `format` over the channel-level `format`. A `stream` channel with no effects writing to it is still spawned for the run's duration, but only as an output source (e.g. tailing a log file) - its stdout/stderr lines still become `Output` events, just with no associated effect.

### Schema types (all in `rngo/src/effect/schema/`)

`Array`, `Constant`, `Context`, `Custom`, `Function`, `Number`, `Object`, `Reference`, `Select`, `Str` (module `string.rs`). Each implements `SchemaBuilder` (parse-time) and `Schema` (run-time). `Custom` backs the spec's `schemas:` section, letting effects reference named custom schema types by name. Builder factory functions are re-exported from `rngo/src/build.rs`.

### Signals & audit (`rngo/src/audit/signal.rs`, `rngo/src/audit.rs`)

Named `signals` in the spec are checks run once, after the simulation completes, against the finished run's SQLite log. The only built-in kind is `SqlSignal` (`rngo/src/audit/signal/sql.rs`): it runs a SQL `query` against the log database and evaluates an optional CEL `expect` expression against the scalar result. `Audit::run()` evaluates every signal and records a `SignalOutcome` (`Success { value, eval }` or `Error`) as run-log metadata; `AuditReport::passed()` is false if any signal fails its expectation or errors, and drives the CLI's process exit status.

### CEL & moments (`rngo/src/cel.rs`, `rngo/src/moment.rs`)

`cel.rs` defines the spec's expression language: `CelContextExt` adds the functions and variables available to spec expressions (`with_time`, `with_hertz`, `with_strings`, `with_now`, `with_simulation`, `with_offset`), and `json_to_cel` converts JSON values into CEL values. It is used by clocks, `Function` schemas, SQL signals, and `MomentParser`. `moment.rs` defines `Moment` (an absolute timestamp or an offset from now), used for simulation and effect `start`/`end`, plus `MomentParser`, which parses RFC 3339, `YYYY-MM-DD`, or CEL expressions into a `Moment`.

### Log (`rngo/src/log.rs`)

A shared `Rc<dyn RunLogReader>` is threaded through all effects and schemas so that `Reference`, trigger-by-effect, and SQL signals can look up previously emitted events — by last input overall, last/random/unique input for a given effect key, or an arbitrary `query()`. A separate `RunLogWriter` trait pushes `Input`, `Output`, and `Metadata` rows. `SimpleEventRunLog` (`log/simple.rs`) is the in-memory implementation; `SqliteRunLog` (`log/sqlite.rs`) persists all three to `log.sqlite` in the run directory and implements both traits.

`InputPool` (`log/pool.rs`) backs the per-effect lookups in both implementations: each effect's inputs in push order, plus a Fenwick tree per (effect, cursor) of consumed positions. `random_for_effect` indexes into the effect's list directly, and `unique_for_effect` finds the k-th unconsumed input in O(log n); `SimpleEventRunLog` also uses it for `last_for_effect`. It assumes inputs for an effect are pushed in increasing `id` order (true because ids come from `last().id + 1`), so "k-th in push order" matches the original `ORDER BY id` selection and seeded runs stay reproducible. `SqliteRunLog` stores only ids in its pool and fetches rows by `id`; it fills the pool from `push_input`, rebuilds it from the `inputs` table and `_unique_reference` metadata rows on open, and still writes those rows on every unique draw.

### Benchmarks (`crates/rngo/benches/`)

Criterion benches, run with `just bench`. The lib target sets `bench = false` so criterion flags (e.g. `--save-baseline`) aren't passed to libtest. Reports land in `target/criterion/`.
- `sqlite_log`: micro benches for `SqliteRunLog` against a pre-filled log (1k/10k inputs split across effects `a` and `b`): `push_input` (write + commit throughput), `last_for_effect`, `random_for_effect`, `unique_for_effect`, and an ad-hoc `query`. `unique_for_effect` consumes inputs as it draws, so it rotates to a fresh cursor every `size / 4` draws to avoid exhausting the pool and measuring the `None` path.
- `simulation`: end-to-end runs of a user/post spec (post references user) with `random` and `unique` cursors, each on both `SimpleEventRunLog` (`memory`) and `SqliteRunLog` (`sqlite`), capped with `limit`. Setup (spec parsing, temp dir, log creation) is excluded from timing; the SQLite variant includes `finish()` and the final `commit()`.
