# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Coding

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
```

## Architecture

The workspace has two crates:
- `crates/rngo` (`rngo`) — core simulation library, published as the public library crate
- `crates/cli` (`rngo-cli`) — CLI binary that wires the library to the filesystem and subprocesses

### Data flow

1. **Spec** (`spec.rs`): A YAML/JSON document loaded from `.rngo/spec.yml` merged with per-file `effects/*.yml`, `channels/*.yml`, `schemas/*.yml`, and `signals/*.yml` (each merge step lives in `cli/src/run.rs::load_spec`, keyed by file stem). Defines `seed`, `start`, `end`, named `effects`, named `channels`, named custom `schemas`, and named `signals`.

2. **Dialect** (`rngo/src/parse/dialect.rs`): Converts a `spec::Spec` into builders by dispatching each effect's schema to a matching `SchemaParser`, each channel's format to a `FormatParser`, each channel's target to a `ChannelTargetParser`, and each signal to a `SignalParser`. `Dialect::primitive()` registers all built-in parsers. Three entry points, one per builder:
   - `parse_merge_effect` → `MergeEffectBuilder`
   - `parse_proxy` → `ProxyBuilder` (channel dispatch)
   - `parse_audit` → `AuditBuilder` (signal evaluation)

3. **MergeEffect** (`effect/merge.rs`): An `Iterator<Item = Input>`. Each call to `next()` sorts all `Effect`s by their next timestamp offset, advances the earliest one, and pushes the resulting `Input` (or `SkippedInput` metadata) to a `RunLogWriter`.

4. **Effect** (`effect.rs`): Also an iterator, yielding `Result<Input, SkippedInput>`. Driven by a `Trigger` (either a `Clock` for time-based firing or another `Effect` for dependency-based firing) and a `Schema` for generating values. `Input` (`{ id, effect, offset, timestamp, data, metadata }`) is the event an effect produces each time it fires.

5. **Proxy** (`rngo/src/proxy.rs`): Channel dispatch. For each `Input`, looks up its effect's assigned channel, formats it via the channel's optional `Format`, and hands it to the channel's `ChannelTarget`, pushing any resulting `Output`s to the `RunLogWriter`.

6. **Audit** (`rngo/src/audit.rs`): Runs after the simulation finishes. Evaluates every named `Signal` against the completed run's log, writes each `SignalOutcome` back as metadata, and produces an `AuditReport` (pass/fail/error counts, `passed()`) that the CLI uses for its exit status.

### CLI run loop (`cli/src/run.rs`)

- `load_spec` merges `.rngo/spec.yml` with `.rngo/effects/*.yml`, `channels/*.yml`, `schemas/*.yml`, `signals/*.yml`, or `load_spec_file` loads a single file when `--spec` is passed.
- `Dialect::primitive()` parses the spec three ways (merge effect, proxy, audit builders).
- `--dry-run`: only builds the `MergeEffect` (to validate the spec) and returns, without creating a run directory or touching channels.
- Otherwise: creates a run directory at `.rngo/runs/<UUIDv7>/`, symlinks `.rngo/runs/last` to it, writes a `spec.json` snapshot, and opens a `SqliteRunLog` (backed by `log.sqlite`) as both the merge effect's `RunLogReader`/`RunLogWriter` and the proxy's writer — wrapped in `StatusWriter` (`cli/src/run/status.rs`), which renders a live effect/output counter to stderr as it forwards writes through.
- Drives the `MergeEffect` iterator, sending each `Input` through the `Proxy`, then calls `proxy.finish()`, then builds and runs the `Audit` against the same `SqliteRunLog` and prints per-signal outcomes.
- `--stdout`: builds the `Proxy` with `stdout(true)`, which swaps every channel's target for a `Stdout` target (prints each input's formatted data to stdout) instead of running the real channel targets.
- `--limit N` caps the total number of effect attempts (successful + skipped) the simulation will produce.

### Channel targets (`rngo/src/channel/target/`)

`ChannelTarget` implementations, wired up by `Proxy`:
- `stream`: spawns one long-lived subprocess per channel, writes formatted event lines to its stdin.
- `exec`: runs a fresh `sh -c <command>` per event; the command string is a Handlebars template rendered with the event's JSON value.
- `stdout`: prints formatted event data to stdout; used in place of the real target when `--stdout` is passed.

An effect opts into a channel by setting `channel: <channel-key>`. The format used is resolved by merging the effect-level `format` over the channel-level `format`. A `stream` channel with no effects writing to it is still spawned for the run's duration, but only as an output source (e.g. tailing a log file) - its stdout/stderr lines still become `Output` events, just with no associated effect.

### Schema types (all in `rngo/src/effect/schema/`)

`Array`, `Constant`, `Context`, `Custom`, `Function`, `Number`, `Object`, `Reference`, `Select`, `Str` (module `string.rs`). Each implements `SchemaBuilder` (parse-time) and `Schema` (run-time). `Custom` backs the spec's `schemas:` section, letting effects reference named custom schema types by name. Builder factory functions are re-exported from `rngo/src/build.rs`.

### Signals & audit (`rngo/src/signal.rs`, `rngo/src/audit.rs`)

Named `signals` in the spec are checks run once, after the simulation completes, against the finished run's SQLite log. The only built-in kind is `SqlSignal` (`rngo/src/signal/sql.rs`): it runs a SQL `query` against the log database and evaluates an optional CEL `expect` expression against the scalar result. `Audit::run()` evaluates every signal and records a `SignalOutcome` (`Success { value, eval }` or `Error`) as run-log metadata; `AuditReport::passed()` is false if any signal fails its expectation or errors, and drives the CLI's process exit status.

### Log (`rngo/src/run_log.rs`)

A shared `Rc<dyn RunLogReader>` is threaded through all effects and schemas so that `Reference`, trigger-by-effect, and SQL signals can look up previously emitted events — by last input overall, last/random/unique input for a given effect key, or an arbitrary `query()`. A separate `RunLogWriter` trait pushes `Input`, `Output`, and `Metadata` rows. `SimpleEventRunLog` (`run_log/simple.rs`) is the in-memory implementation; `SqliteRunLog` (`run_log/sqlite.rs`) persists all three to `log.sqlite` in the run directory and implements both traits.
