# Changelog

Each release has a section named after its version. Changes that break existing specs say how to update them; the full spec migration guide is in [MIGRATING.md](MIGRATING.md).

## Unreleased

### Added

- A spec declares its format in a top-level `version` (currently 1), and `rngo` reports a spec in a format it doesn't read. JSON Schemas for editors are in `schema/v1/`, and `rngo init` adds the `$schema` line to a new `spec.yml`.
- `rngo init` installs the agent skills release that matches the CLI version.
- `exec` and `stream` channels work on Windows.
- The docs on docs.rs include items behind the `schema` feature, such as `spec::json_schema`, marked with the feature they need.

### Changed

- **Breaking (spec):** keys from before versioning are rejected with a pointer to [MIGRATING.md](MIGRATING.md). Move `phases`, `schemas` and a channel's `target` as described there.
- **Breaking (Rust API):** the public API of the `rngo` crate is smaller. Only the items listed in the crate README are public, and public enums and spec structs are `#[non_exhaustive]`, so match them with a wildcard arm.
- **Breaking (Rust API):** `rngo` and `rngo-cli` use `jiff` instead of the unmaintained `chrono` (RUSTSEC-2026-0543). Timestamps (`Input::timestamp`, `Moment::Absolute`, `now`, `start`/`end`) are `jiff::Timestamp` instead of `DateTime<FixedOffset>`, and durations (`Moment::Relative`) are `jiff::SignedDuration` instead of `chrono::TimeDelta`. Timestamps in JSON and `context` generator output are written as UTC with a `Z` suffix (`2024-01-02T00:00:00Z`) instead of `+00:00`, and without trailing fractional zeros.
- An invalid spec is reported as an error instead of panicking. A `number` generator without the bounds it needs is a parse error, and a trigger expression that stops returning a non-negative number ends the run with an error after logging what ran.
- A `rngo-cli` build with a bumped `zip` fixes RUSTSEC-2026-0546, 0547 and 0548.

## 0.40.0 - 2026-11-10

### Added

- Prototypes: named defaults for `start`, `end`, `limit` and `channel` that an effect opts into with its `prototype` field. Expressions can refer to the parent prototype as `prototype.start` and `prototype.end`.
- Scenarios: `rngo eval --scenario <name>` patches the prototypes that define it. The patched spec is saved as `spec.rewritten.json` in the run directory.

### Changed

- **Breaking (CLI):** the spec is a positional argument of `rngo eval` (a file, a `.rngo` directory or a directory that contains one), replacing the `--spec` and `--dir` flags. Run `rngo eval path/to/spec`.
- Report a clear error when a spec file isn't a YAML mapping.
- Report the exit status and stderr when a `stream` channel's command dies.

## 0.39.0 - 2026-10-22

### Changed

- **Breaking (CLI):** `rngo run` is now `rngo eval`.
- **Breaking (spec):** `schemas` is renamed to `generators`.
- **Breaking (spec):** a channel is flat. Put the target's fields (`type`, `command`, ...) directly on the channel next to `format`, and remove the nested `target`.
- **Breaking (Rust API):** `ParseError::SchemaError` is now `ParseError::Invalid`, and the channel wrapper types are gone.
- The CLI calls generated events "inputs" in its status and help text.

## 0.38.0 - 2026-10-05

### Added

- `template` format: renders a Handlebars template against the whole input, with a `json` helper.
- `--realtime` holds each input until its timestamp has passed; `--stdout` is paced the same way. Ctrl-C stops the run and still audits what was logged, and a second Ctrl-C exits immediately. SIGTERM is handled like the first Ctrl-C, and an interrupted run exits non-zero.

### Changed

- **Breaking (spec):** `exec` channels run each input's formatted data as the command, so they require a `format` (usually `template`), and a `command` field is a parse error. In templates, the effect is `effect.key`.
- **Breaking (run log):** timestamps are epoch milliseconds in a single `timestamp` column instead of an offset plus an RFC 3339 timestamp, including in `outputs` and for skipped attempts. Update SQL signals that read them.
- Future inputs are sent right away unless `--realtime` is passed.
- A format failure records an error output for that input instead of falling back to the raw data.
