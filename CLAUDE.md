# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Commands

```bash
cargo test --workspace          # run all tests
cargo test -p rngo-sim          # run sim crate tests only
cargo test <test_name>          # run a single test by name
cargo fmt                       # format code (rustfmt.toml: imports_granularity = "Module")
cargo fmt --check               # check formatting (used in CI)
cargo clippy --workspace --all-targets -- -D warnings  # lint (warnings are errors in CI)
cargo build                     # build
cargo run -p rngo-cli -- sim run            # run simulation (writes to .rngo/runs/local/<N>/)
cargo run -p rngo-cli -- sim run --stdout   # run simulation, print all events to stdout as JSON
```

## Architecture

The workspace has two crates:
- `crates/sim` (`rngo-sim`) — core simulation library
- `crates/cli` (`rngo-cli`) — CLI binary that wires the library to the filesystem and subprocesses

### Data flow

1. **Spec** (`spec.rs`): A YAML/JSON document loaded from `.rngo/spec.yml` + per-file `effects/*.yml` and `systems/*.yml`. Defines `seed`, `start`, `end`, named `effects`, and named `systems`.

2. **Dialect::parse_simulation** (`spec/parse.rs`): Converts a `spec::Simulation` into a `SimulationBuilder` by dispatching each effect's schema to a matching `SchemaParser` and each effect's format to a matching `FormatParser`. `Dialect::core()` registers all built-in parsers.

3. **Simulation** (`simulation.rs`): An `Iterator<Item = Event>`. Each call to `next()` sorts all `Effect`s by their next timestamp offset and advances the earliest one.

4. **Effect** (`effect.rs`): Also an `Iterator<Item = Event>`. Driven by a `Trigger` (either a `Clock` for time-based firing or another `Effect` for dependency-based firing) and a `Schema` for generating values.

5. **Event** (`event.rs`): Either `Event::Effect { id, key, offset, value, format }` or `Event::Error { id, message }`.

### CLI run loop (`cli/src/sim/run.rs`)

- Loads spec, creates a run directory at `.rngo/runs/local/<N>/`, writes `spec.json` snapshot.
- Without `--stdout`: writes each `Event::Effect` as a JSON line to `<effect-key>.jsonl` and dispatches to any assigned system via `SystemDispatch`.
- With `--stdout`: serializes all events (including errors) to stdout.

### System imports (`cli/src/sim/system.rs`)

`SystemDispatch` implements two integration modes for `systems`:
- `stream`: spawns one long-lived subprocess per system, writes formatted event lines to its stdin.
- `exec`: runs a fresh `sh -c <command>` per event; the command string is a Handlebars template rendered with the event's JSON value.

An effect opts into a system by setting `system: <system-key>`. The format used is resolved by merging the effect-level `format` over the system-level `format`.

### Schema types (all in `sim/src/schema/`)

`Array`, `Constant`, `Context`, `Function`, `Number`, `Object`, `Reference`, `Select`, `Str`. Each implements `SchemaBuilder` (parse-time) and `Schema` (run-time). Builder factory functions are re-exported from `sim/src/build.rs`.

### EventLog (`sim/src/event/log.rs`)

A shared `Rc<dyn EventLog>` is threaded through all effects so that `Reference` and trigger-by-effect can look up previously emitted events. `SimpleEventLog` is the only implementation.
