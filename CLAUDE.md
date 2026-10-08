# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Coding

Give interfaces (traits, trait methods, public structs, enums, and functions) terse `///` doc comments: one line on what it does, plus what each non-obvious argument means. Skip ones that just restate the name or signature.

Avoid inline comments. If additional context is needed for a section of code, add it in CLAUDE.md.

The Rust toolchain is pinned in `rust-toolchain.toml` (used locally and in CI), and the minimum supported Rust version is `rust-version` under `[workspace.package]` in the root `Cargo.toml`. Bump them together, in their own PR, and fix any new clippy lints there.

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
just memory                     # peak heap of SQLite-backed runs at 100k and 1M inputs
just memory 10_000_000          # peak heap at specific input counts
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

3. **Simulation** (`effect/simulation.rs`): An `Iterator<Item = Result<Input, SkippedInput>>`, the same item type as `Effect`. Each call to `next()` sorts all `Effect`s by their next offset and advances the earliest one, yielding its result as is, so a skipped attempt is an `Err` item rather than being looped past. It requires a `RunLogReader` (so effects can look up earlier inputs; `build` errors without one, rather than defaulting) but has no writer: it logs nothing, and its `limit` counts items of either kind. Whatever consumes it must write the inputs it yields back to that log before calling `next()` again, since effects read the previous input from it (the `Proxy` does this). `StandaloneSimulation` (same file) is the alternative that logs for itself: it holds a `Simulation` and a `SimpleEventRunLog`, writes each yielded input to it (not skipped attempts), and is built by `SimulationBuilder::standalone()`, which creates the log and sets it as the reader. Orchestrates many `Effect`s but doesn't share a trait with it — `Effect` models a single logical kind of input; `Simulation` merges many of them into one time-ordered run.

4. **Effect** (`effect.rs`): Also an iterator, yielding `Result<Input, SkippedInput>`. Driven by a `Trigger` (either a `Clock` for time-based firing or another `Effect` for dependency-based firing) and a `Schema` for generating values. `Input` (`{ id, effect, timestamp, data, metadata }`) is the event an effect produces each time it fires.

5. **Proxy** (`rngo/src/proxy.rs`, with `channel`, `format`, and `output` submodules under `rngo/src/proxy/`): Channel dispatch. For each `Input`, waits for it in realtime mode, then writes it to the `RunLogWriter` (even when its effect has no channel, since other effects may reference it) just before looking up its effect's assigned channel, formats it via the channel's optional `Format`, and hands it to the channel's `ChannelTarget`, pushing any resulting `Output`s to the `RunLogWriter`. It also records the `timing` metadata rows whose `data` is `{ "key", "timestamp" }` (wall-clock epoch milliseconds): `simulation_start` on the first `send`, and `simulation_end` on `finish()`, so signals can read both (a run that sends no input has neither). With `ProxyBuilder::realtime(true)`, `send` first blocks until the input's timestamp has passed, calling the optional `on_wait` hook when a wait starts and about once a second during it. A `StopHandle` (`proxy/stop.rs`; from `Proxy::stop_handle()`, or shared in via `ProxyBuilder::stop_handle`) is the only way to stop a proxy: it is `Send + Sync` so another thread can call `stop()`, which wakes any wait immediately (a condvar) and makes later `send`s no-ops. It can't finish the proxy (`Proxy` isn't `Send`), so the owner still calls `finish()`. Realtime sending lives here rather than in `Simulation` on purpose: the simulation generates each input at its logical time, ahead of when it is actually sent, and the proxy logs an input only once it is due, so an input a stop interrupts is never logged.

6. **Audit** (`rngo/src/audit.rs`): Runs after the simulation finishes. Evaluates every named `Signal` against the completed run's log, writes each `SignalOutcome` back as metadata, and produces an `AuditReport` (pass/fail/error counts, `passed()`) that the CLI uses for its exit status.

### CLI run loop (`cli/src/run.rs`)

- `load_spec` merges `.rngo/spec.yml` with `.rngo/effects/*.yml`, `channels/*.yml`, `schemas/*.yml`, `signals/*.yml`, or `load_spec_file` loads a single file when `--spec` is passed.
- `Dialect::primitive()` parses the spec three ways (simulation, proxy, audit builders).
- `--dry-run`: only builds the `Simulation` (to validate the spec) and returns, without creating a run directory or touching channels.
- Otherwise: creates a run directory at `.rngo/runs/<UUIDv7>/`, symlinks `.rngo/runs/last` to it, writes a `spec.json` snapshot, and opens a `SqliteRunLog` (backed by `log.sqlite`) as both the simulation's `RunLogReader`/`RunLogWriter` and the proxy's writer — wrapped in `StatusWriter` (`cli/src/run/status.rs`), which renders a live effect/output counter to stderr as it forwards writes through. `StatusWriter::finish()` is called right after `proxy.finish()` so the final redraw happens before the audit prints to stdout; its in-place redraw clears the last N lines, so redrawing after other output would erase that output.
- Only when `--realtime` is passed, builds the proxy with `realtime(true)` and an `on_wait` hook that flushes the run log (`RunLogWriter::flush`; `SqliteRunLog` otherwise commits in batches, so a long wait or a hard exit would leave rows uncommitted) and shows a countdown via `StatusWriter::wait`. `--stdout` is paced the same way. Without `--realtime`, future inputs are sent right away.
- Ctrl-C and SIGTERM are handled in `main.rs` (`ctrlc` with `termination`): the first calls `stop()` on the `RunOptions::stop` handle, which is shared into the proxy, ending any wait; the run loop then breaks, so the run finishes and audits, but `run` returns `Ok(false)` (exit status 1) because it was stopped; a second exits immediately with status 130.
- Drives the `Simulation` iterator, sending each `Ok` input through the `Proxy` and writing each skipped attempt (`Err`) to the run log as `skipped` metadata itself, then calls `proxy.finish()`, then builds and runs the `Audit` against the same `SqliteRunLog` and prints per-signal outcomes.
- `--stdout`: builds the `Proxy` with `stdout(true)`, which swaps every channel's target for a `Stdout` target (prints each input's formatted data to stdout) instead of running the real channel targets.
- `--limit N` caps the total number of effect attempts (successful + skipped) the simulation will produce.

### Channel targets (`rngo/src/proxy/channel/target/`)

`ChannelTarget` implementations, wired up by `Proxy`. `send` gets only a `serde_json::Value`, not the `Input`: the formatted data as a string when the channel has a format, otherwise the input's own `data`. Targets never build `Output`s: they report `TargetOutput`s (`level` + `message`, in `proxy/output.rs`), and the proxy side converts them. `Proxy::send` turns the ones `send` returns into `Output`s tied to the input and channel. Outputs a target reports later go through the `OutputSender` it gets at build time (e.g. `stream`'s reader threads), which stamps the channel and the time of sending and ties them to no input.
- `stream`: spawns one long-lived subprocess per channel, writes one line per event to its stdin (a string value as-is, anything else as JSON).
- `exec`: runs a fresh `sh -c <data>` per event. A non-string value records an error `Output` for that input instead of running. A `command` field on the target is rejected at parse time.
- `stream` and `exec` spawn through `target::shell`, which puts the child in its own process group on Unix so a terminal Ctrl-C reaches rngo but not the child, letting `proxy.finish()` drain its output after an early stop.
- `stdout`: prints each event's value to stdout (a string as-is, anything else as JSON); used in place of the real target when `--stdout` is passed.

An effect opts into a channel by setting `channel: <channel-key>`. Formats are configured only on channels; effects know nothing about formats.

### Formats (`rngo/src/proxy/format/`)

- `sql`: renders each event as an `INSERT` statement.
- `template`: renders a Handlebars `template` against the whole serialized `Input` (`id`, `effect.key`, `timestamp`, `data`, `metadata`), with HTML escaping off and a `json` helper that serializes its one argument.

If a format fails for an event, `Proxy::send` pushes an error-level `Output` for that input and channel, skips the target, and carries on with the run. Format and target parse errors are prefixed with `channels.<key>.format` / `channels.<key>.target` in `Dialect::parse_proxy`, so parsers return paths relative to their own node. A `stream` channel with no effects writing to it is still spawned for the run's duration, but only as an output source (e.g. tailing a log file) - its stdout/stderr lines still become `Output` events, just with no associated effect.

### Schema types (all in `rngo/src/effect/schema/`)

`Array`, `Constant`, `Context`, `Custom`, `Function`, `Number`, `Object`, `Reference`, `Select`, `Str` (module `string.rs`). Each implements `SchemaBuilder` (parse-time) and `Schema` (run-time). `Custom` backs the spec's `schemas:` section, letting effects reference named custom schema types by name. Builder factory functions are re-exported from `rngo/src/build.rs`.

### Signals & audit (`rngo/src/audit/signal.rs`, `rngo/src/audit.rs`)

Named `signals` in the spec are checks run once, after the simulation completes, against the finished run's SQLite log. The only built-in kind is `SqlSignal` (`rngo/src/audit/signal/sql.rs`): it runs a SQL `query` against the log database and evaluates an optional CEL `expect` expression against the scalar result. `Audit::run()` evaluates every signal and records a `SignalOutcome` (`Success { value, eval }` or `Error`) as run-log metadata; `AuditReport::passed()` is false if any signal fails its expectation or errors, and drives the CLI's process exit status.

### CEL & moments (`rngo/src/cel.rs`, `rngo/src/moment.rs`)

`cel.rs` defines the spec's expression language: `CelContextExt` adds the functions and variables available to spec expressions (`with_time`, `with_hertz`, `with_strings`, `with_now`, `with_simulation`, `with_offset`), and `json_to_cel` converts JSON values into CEL values. It is used by clocks, `Function` schemas, SQL signals, and `MomentParser`. `moment.rs` defines `Moment` (an absolute timestamp or an offset from now), used for simulation and effect `start`/`end`, plus `MomentParser`, which parses RFC 3339, `YYYY-MM-DD`, or CEL expressions into a `Moment`.

### Log (`rngo/src/log.rs`)

A shared `Rc<dyn RunLogReader>` is threaded through all effects and schemas so that `Reference`, trigger-by-effect, and SQL signals can look up previously emitted events — by last input overall, last/random/unique input for a given effect key, or an arbitrary `query()`. A separate `RunLogWriter` trait pushes `Input`, `Output`, and `Metadata` rows. `SimpleEventRunLog` (`log/simple.rs`) is the in-memory implementation; `SqliteRunLog` (`log/sqlite.rs`) persists all three to `log.sqlite` in the run directory and implements both traits.

`InputPool` (`log/pool.rs`) backs the per-effect lookups in both implementations: each tracked effect's inputs in push order, plus a consumed set per (effect, cursor). Effects are tracked lazily: the first lookup for an effect loads its already-logged inputs (each log's private `pool(effect)` helper does this), and `push` ignores untracked effects, so effects nothing references cost no memory. A consumed set is a bitset of positions plus a Fenwick tree with one node per 64-bit word; a lookup walks the tree to the word, then picks the zero bit within it. The pool owns the random draws: `random` (for `random_for_effect`) indexes into the effect's list, and `take` (for `unique_for_effect`) finds the k-th unconsumed input in O(log n); `SimpleEventRunLog` also uses it for `last_for_effect`. Because the draws live in the pool, both logs produce identical output for a given seed. Selection only needs to be deterministic for a given seed and version, not stable across versions (the project is pre-1.0). A consumed set's state depends only on which inputs a cursor has consumed, not the order it consumed them in, so rebuilding it from the log reproduces an uninterrupted run exactly. `mark` finds an item by binary search, which relies on each effect's inputs being pushed in increasing `id` order (true because ids come from `last().id + 1`). `SqliteRunLog` stores only ids in its pool and fetches rows by `id`; when it first tracks an effect it loads that effect's ids from the `inputs` table and replays its `_unique_reference` metadata rows, which also restores cursor state when reopening an existing log. It still writes those rows on every unique draw.

### Benchmarks (`crates/rngo/benches/`)

Criterion benches, run with `just bench`. The lib target sets `bench = false` so criterion flags (e.g. `--save-baseline`) aren't passed to libtest. Reports land in `target/criterion/`.
- `sqlite_log`: micro benches for `SqliteRunLog` against a pre-filled log (1k/10k inputs split across effects `a` and `b`): `push_input` (write + commit throughput), `last_for_effect`, `random_for_effect`, `unique_for_effect`, and an ad-hoc `query`. `unique_for_effect` consumes inputs as it draws, so it rotates to a fresh cursor every `size / 4` draws to avoid exhausting the pool and measuring the `None` path.
- `crates/rngo/examples/memory.rs` (`just memory`, not criterion): runs a spec with a referenced `user` effect (unique and random cursors on it) and a high-volume unreferenced `event` effect on `SqliteRunLog`, and reports peak Rust heap via a counting global allocator. SQLite's own C allocations aren't counted, which isolates the pool.
- `simulation`: end-to-end runs of a user/post spec (post references user) with `random` and `unique` cursors, each on both `SimpleEventRunLog` (`memory`) and `SqliteRunLog` (`sqlite`), capped with `limit`. Setup (spec parsing, temp dir, log creation) is excluded from timing; the SQLite variant includes `finish()` and the final `commit()`.
