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
cargo run -p rngo-cli -- eval            # evaluate simulation (writes to .rngo/evals/<UUID>/, symlinked from .rngo/evals/last)
cargo run -p rngo-cli -- eval --stdout   # evaluate simulation, print all events to stdout as JSON
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

`rngo-cli` is a library (`src/lib.rs`: `eval`, `init`, `skills`) plus a thin `main.rs` that only parses arguments and handles signals, so `crates/cli/tests/` can call `eval()` directly with custom `EvalOptions`. End-to-end `eval` tests live in `tests/eval.rs`; unit tests of private code stay in their modules.

### Data flow

1. **Spec** (`spec.rs`): A YAML/JSON document loaded from `.rngo/spec.yml` merged with per-file `effects/*.yml`, `channels/*.yml`, `schemas/*.yml`, and `signals/*.yml` (each merge step lives in `cli/src/eval.rs::load_spec`, keyed by file stem). Defines `seed`, `start`, `end` (the default phase), named `phases`, named `effects`, named `channels`, named custom `schemas`, and named `signals`.

2. **Dialect** (`rngo/src/parse/dialect.rs`): Converts a `spec::Spec` into builders by dispatching each effect's schema to a matching `SchemaParser`, each channel's `type` to a `ChannelParser` (which parses the channel's `format` with the dialect's `FormatParser`s), and each signal to a `SignalParser`. `Dialect::primitive()` registers all built-in parsers. Three entry points, one per builder:
   - `parse_simulation` → `SimulationBuilder`
   - `parse_proxy` → `ProxyBuilder` (channel dispatch)
   - `parse_audit` → `AuditBuilder` (signal evaluation)

   Phases (`spec::Phase`) are named time windows. In the library, `effect::scenario::Scenario` (from `ScenarioBuilder::build(now)`) owns the seed, `now`, the default phase (the spec's top-level `start`/`end`) and the named `effect::phase::Phase`s. `PhaseBuilder::build(now)` resolves a phase to instants (erroring if `start` is not before `end`); whichever of `start`/`end` it leaves unset comes from its parent (the default phase; the last fallback is -30d/now). `SimulationBuilder` knows none of this: it takes a run log reader, a limit and effects added with `with_effect`. `parse_simulation` fills a `ScenarioBuilder` from the spec but builds nothing: it assigns each effect to a phase (a phase's `effects` list beats its `channels`; otherwise the default phase), and gives every effect the same `Rc<ScenarioBuilder>` (`EffectBuilder::set_scenario`) and its phase name (`set_phase`); the effect's `start`/`end` (a `Moment` or, from the spec, an expression) go to `Phase::child_builder().for_effect(key)`, which parses expressions with the phase's bounds as `simulation`, defaults to the phase's bounds and errors if the window leaves the phase. `SimulationBuilder::build` sets one shared `now` on each effect, and the effect builds the scenario with it (`ScenarioBuilder::build(now)`), so an effect given no scenario gets an empty one. `Spec::with_scenario` patches the phases that define a scenario (`rngo eval --scenario`); the CLI applies it before parsing and records the result in `spec.rewritten.json`.

3. **Simulation** (`effect/simulation.rs`): An `Iterator<Item = Result<Input, SkippedInput>>`, the same item type as `Effect`. Each call to `next()` sorts all `Effect`s by their next offset and advances the earliest one, yielding its result as is, so a skipped attempt is an `Err` item rather than being looped past. Effects measure offsets from one shared origin (the earliest phase start) while their bounds, and the `GeneratorContext` start/end, are their phase's. It requires a `RunLogReader` (so effects can look up earlier inputs; `build` errors without one, rather than defaulting) but has no writer: it logs nothing, and its `limit` counts only `Ok` inputs, so an effect that always skips runs until the time window ends. Whatever consumes it must write the inputs it yields back to that log before calling `next()` again, since effects read the previous input from it (the `Proxy` does this). `StandaloneSimulation` (same file) is the alternative that logs for itself: it holds a `Simulation` and a `SimpleEventRunLog`, writes each yielded input to it (not skipped attempts), and is built by `SimulationBuilder::standalone()`, which creates the log and sets it as the reader. Orchestrates many `Effect`s but doesn't share a trait with it — `Effect` models a single logical kind of input; `Simulation` merges many of them into one time-ordered run.

4. **Effect** (`effect.rs`): Also an iterator, yielding `Result<Input, SkippedInput>`. Driven by a `Trigger` (either a `Clock` for time-based firing or another `Effect` for dependency-based firing) and a `Schema` for generating values. `Input` (`{ id, effect, timestamp, data, metadata }`) is the event an effect produces each time it fires.

5. **Proxy** (`rngo/src/proxy.rs`, with `channel`, `format`, and `output` submodules under `rngo/src/proxy/`): Channel dispatch. For each `Input`, waits for it in realtime mode, then writes it to the `RunLogWriter` (even when its effect has no channel, since other effects may reference it) just before looking up its effect's assigned channel and handing it to the channel (a `Channel`, which formats it), pushing any resulting `Output`s to the `RunLogWriter`. It also records the `timing` metadata rows whose `data` is `{ "key", "timestamp" }` (wall-clock epoch milliseconds): `simulation_start` on the first `send`, and `simulation_end` on `finish()`, so signals can read both (a run that sends no input has neither). With `ProxyBuilder::realtime(true)`, `send` first blocks until the input's timestamp has passed, calling the optional `on_wait` hook when a wait starts and about once a second during it. A `StopHandle` (`proxy/stop.rs`; from `Proxy::stop_handle()`, or shared in via `ProxyBuilder::stop_handle`) is the only way to stop a proxy: it is `Send + Sync` so another thread can call `stop()`, which wakes any wait immediately (a condvar) and makes later `send`s no-ops. It can't finish the proxy (`Proxy` isn't `Send`), so the owner still calls `finish()`. Realtime sending lives here rather than in `Simulation` on purpose: the simulation generates each input at its logical time, ahead of when it is actually sent, and the proxy logs an input only once it is due, so an input a stop interrupts is never logged.

6. **Audit** (`rngo/src/audit.rs`): Runs after the simulation finishes. Evaluates every named `Signal` against the completed run's log, writes each `SignalOutcome` back as metadata, and produces an `AuditReport` (pass/fail/error counts, `passed()`) that the CLI uses for its exit status.

### CLI eval loop (`cli/src/eval.rs`)

- `resolve_source` takes the optional positional path: a file is the whole spec (`load_spec_file`, nothing else merged), a directory named `.rngo` is built from by `load_spec`, any other directory uses the `.rngo` inside it, and no path means `./.rngo`. `load_spec` merges `spec.yml` with `effects/*.yml`, `channels/*.yml`, `generators/*.yml`, `signals/*.yml`. `.env` is read from the directory containing the `.rngo` directory (the current directory for a file), and evals are written to `<rngo dir>/evals` (`./.rngo/evals` for a file).
- `Dialect::primitive()` parses the spec three ways (simulation, proxy, audit builders).
- `--dry-run`: only builds the `Simulation` (to validate the spec) and returns, without creating a run directory or touching channels.
- Otherwise: creates an eval directory at `.rngo/evals/<UUIDv7>/`, symlinks `.rngo/evals/last` to it, writes a `spec.json` snapshot, and opens a `SqliteRunLog` (backed by `log.sqlite`) as both the simulation's `RunLogReader`/`RunLogWriter` and the proxy's writer — wrapped in `StatusWriter` (`cli/src/eval/status.rs`), which renders a live effect/output counter to stderr as it forwards writes through. `StatusWriter::finish()` is called right after `proxy.finish()` so the final redraw happens before the audit prints to stdout; its in-place redraw clears the last N lines, so redrawing after other output would erase that output.
- Only when `--realtime` is passed, builds the proxy with `realtime(true)` and an `on_wait` hook that flushes the run log (`RunLogWriter::flush`; `SqliteRunLog` otherwise commits in batches, so a long wait or a hard exit would leave rows uncommitted) and shows a countdown via `StatusWriter::wait`. `--stdout` is paced the same way. Without `--realtime`, future inputs are sent right away.
- Ctrl-C and SIGTERM are handled in `main.rs` (`ctrlc` with `termination`): the first calls `stop()` on the `RunOptions::stop` handle, which is shared into the proxy, ending any wait; the run loop then breaks, so the run finishes and audits, but `run` returns `Ok(false)` (exit status 1) because it was stopped; a second exits immediately with status 130.
- Drives the `Simulation` iterator, sending each `Ok` input through the `Proxy` and writing each skipped attempt (`Err`) to the run log as `skipped` metadata itself, then calls `proxy.finish()`, then builds and runs the `Audit` against the same `SqliteRunLog` and prints per-signal outcomes.
- `--stdout`: parses the proxy from a spec whose channels are all `stdout`, after rewriting every channel to a `stdout` one (see Channels), so inputs print instead of running the real channels.
- `--limit N` caps the number of inputs the simulation will produce; skipped attempts don't count.

### Channels (`rngo/src/proxy/channel.rs` and `channel/`)

`Channel` implementations, one per channel `type` (a spec channel is flat: `type` plus the channel's own fields such as `command` and `format`; a leftover nested `target` is rejected at parse time). `Proxy` holds them in a map by channel key; there is no wrapper struct, so the key and the effects assigned to a channel live on the proxy (`ProxyBuilder::channel` and `channel_effects`). A channel owns its format: its parser builds the optional `format` field through `FormatParseContext` (so a channel that takes no format can reject it), and `send` gets the whole `Input`, delivering the formatted string or, without a format, the input's own `data` (`channel::render`). Channels never build `Output`s: they report `ChannelOutput`s (`level` + `message`, in `proxy/output.rs`), and the proxy side converts them. `Proxy::send` turns the ones `send` returns into `Output`s tied to the input and channel. Outputs a channel reports later go through the `OutputSender` it gets at build time (e.g. `stream`'s reader threads), which stamps the channel and the time of sending and ties them to no input.
- `stream`: spawns one long-lived subprocess per channel, writes one line per event to its stdin (a string value as-is, anything else as JSON).
- `exec`: runs a fresh `sh -c <data>` per event. A non-string value records an error `Output` for that input instead of running. A `command` field on the channel is rejected at parse time.
- `stream` and `exec` spawn through `channel::shell`, which puts the child in its own process group on Unix so a terminal Ctrl-C reaches rngo but not the child, letting `proxy.finish()` drain its output after an early stop.
- `stdout`: prints each event's value to stdout (a string as-is, anything else as JSON). `--stdout` doesn't swap channels in the proxy: `cli/src/eval.rs::to_stdout_channels` rewrites every channel of the loaded spec to `type: stdout`, keeping only its `format`, before the spec is parsed. The run directory then holds the spec as written (`spec.json`) and the rewritten one (`spec.rewritten.json`, only with `--stdout`). A source-only `stream` channel therefore isn't spawned under `--stdout`.

An effect opts into a channel by setting `channel: <channel-key>`. Formats are configured only on channels (and applied by the channel itself); effects know nothing about formats.

### Formats (`rngo/src/proxy/format/`)

- `sql`: renders each event as an `INSERT` statement.
- `template`: renders a Handlebars `template` against the whole serialized `Input` (`id`, `effect.key`, `timestamp`, `data`, `metadata`), with HTML escaping off and a `json` helper that serializes its one argument.

If a format fails for an event, the channel reports an error-level `ChannelOutput` (tied to that input and channel by `Proxy::send`), skips the send, and carries on with the run. Channel parse errors are prefixed with `channels.<key>` in `Dialect::parse_proxy` (format errors are `channels.<key>.format...`), so parsers return paths relative to the channel. A `stream` channel with no effects writing to it is still spawned for the run's duration, but only as an output source (e.g. tailing a log file) - its stdout/stderr lines still become `Output` events, just with no associated effect.

### Errors (`rngo/src/error.rs`)

All error types live in `error.rs` and are re-exported from the crate root (`ParseError` and `BuildError` also from `spec` and `build`). Each phase has its own type, built with `thiserror`:
- `ParseError`: a struct of a `path` (the spec field it concerns, empty for the whole document) and a `ParseErrorKind` (`Invalid` free-form message, `UnknownType`, `AmbiguousType`, `UnknownChannel`). Parsers return `Vec<ParseError>`; build them with `ParseError::invalid(message).at(path)` and nest them with `prefixed(prefix)`, which is how `Dialect::parse_proxy` and `FormatParseContext::parse` add `channels.<key>` and `format`.
- `BuildError`: one variant per part of the run being built (`Simulation`, `Effect`, `Generator`, `Proxy`, `Channel`, `Signal`, `Audit`), each with a `message`. Build them with the lowercase constructors (`BuildError::effect(..)`). Builders return `Vec<BuildError>`.
- `RunLogError`: reading or writing a run log. `RunLogWriter` methods and `RunLogReader::query` return it. The `RunLogReader` lookups return `Option`, so `SqliteRunLog` records a failed lookup and returns it from its next write, `flush` or `commit`; check `commit` after the last lookup.
- `FormatError` (from `Format::format`), `ChannelError` (from `Channel::send`, for failures the run can't continue past, such as a broken pipe) and `ProxyError` (`Proxy::send`, wrapping the channel and run log ones). A format failure isn't fatal: `channel::render` turns it into an error `ChannelOutput`.
- `SignalError`: why a signal couldn't be evaluated. `SqlSignal::evaluate` turns it into a `SignalOutcome::Error`, adding the `signal `<key>`:` prefix in one place.

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
