# Changelog

Each release has a section named after its version. Changes that break existing specs say how to update them; the full spec migration guide is in [MIGRATING.md](MIGRATING.md).

## Unreleased

### Added

- A spec declares its format in a top-level `version` (currently 1), and `rngo` reports a spec in a format it doesn't read. JSON Schemas for editors are in `schema/v1/`, and `rngo init` adds the `$schema` line to a new `spec.yml`.
- `rngo init` installs the agent skills release that matches the CLI version.
- `exec` and `stream` channels work on Windows.
- The docs on docs.rs include items behind the `schema` feature, such as `spec::json_schema`, marked with the feature they need.
- Builder `start` and `end` accept a `jiff::SignedDuration`, relative to now. The crate README writes them as spec expressions, such as `.start("now - days(7)")`.
- A custom generator can skip an attempt with `GeneratorResult::skip()` and attach metadata to any result with `.metadata(..)`, as the built-in generators do.

### Changed

- The `rngo-cli` crate's library is internal: it exists only for the CLI's own tests, is hidden from its docs, and may change in any release. To run simulations from Rust, use the `rngo` crate. The `rngo` command itself doesn't change.
- **Breaking (spec):** keys from before versioning are rejected with a pointer to [MIGRATING.md](MIGRATING.md). Move `phases`, `schemas` and a channel's `target` as described there.
- **Breaking (spec):** an unknown field on an effect, a trigger object, a prototype's scenario or a custom generator is a parse error at that field (e.g. `effects.user.limt`) instead of being ignored. Remove or fix the field. Such a field never had any effect, so the spec `version` stays 1.
- **Breaking (Rust API):** the public API of the `rngo` crate is smaller. Only the items listed in the crate README are public, and public enums and spec structs are `#[non_exhaustive]`, so match them with a wildcard arm.
- **Breaking (Rust API):** `rngo` and `rngo-cli` use `jiff` instead of the unmaintained `chrono` (RUSTSEC-2026-0543). Timestamps (`Input::timestamp`, `Moment::Absolute`, `now`, `start`/`end`) are `jiff::Timestamp` instead of `DateTime<FixedOffset>`, and durations (`Moment::Relative`) are `jiff::SignedDuration` instead of `chrono::TimeDelta`. Timestamps in JSON and `context` generator output are written as UTC with a `Z` suffix (`2024-01-02T00:00:00Z`) instead of `+00:00`, and without trailing fractional zeros.
- **Breaking (Rust API):** `EffectBuilder::limit` and `PrototypeBuilder::limit` take a `u64`, like `SimulationBuilder::limit`. Replace `.limit(NonZeroU64::new(n).unwrap())` with `.limit(n)`. A limit of 0 on any of the three builders is now a build error; `SimulationBuilder::limit(0)` used to produce no inputs.
- **Breaking (Rust API):** the `rngo::parsers` module is renamed `rngo::parse`, matching `rngo::build`. Replace `rngo::parsers::array()` and the like with `rngo::parse::array()`.
- **Breaking (Rust API):** parsing and building return `ParseErrors` and `BuildErrors` instead of `Vec<ParseError>` and `Vec<BuildError>`. Both implement `std::error::Error` and display one error per line, so `?` propagates them into `Box<dyn Error>`, `anyhow::Error` or your own error type. This covers `spec::from_value`, `Dialect::parse_*`, every builder's `build()` and `SimulationBuilder::standalone()`, and the parser and builder traits for custom generators, channels, formats and signals. To migrate, code that reads the errors mostly keeps working, since both types deref to a slice, have `as_slice()` and iterate like a `Vec`; use `into_vec()` for an owned `Vec`. Custom parsers and builders return `Err(error.into())` for one error, or `Err(errors.into())` for a `Vec`.
- **Breaking (Rust API):** `RunError` is renamed `SimulationError`, and its fields are private: read them with `effect()`, `key()` and `message()`. Its message starts with `simulation failed at` instead of `failed to run`. `GeneratorParseVisitor::invalid` and `input_error` are renamed `error` and `field_error`, matching `GeneratorBuildVisitor::error`.
- **Breaking (Rust API):** `Dialect::parse_simulation`, `parse_proxy` and `parse_audit` borrow the spec instead of taking it by value, so parsing one spec three ways no longer clones it. To migrate, pass `&spec` and drop any `spec.clone()` made to call more than one of them.
- **Breaking (Rust API):** `EffectBuilder` and `PrototypeBuilder` only describe an effect or a prototype, and the simulation fills in the rest. `EffectBuilder::prototype`, `now`, `seed`, `event_run_log`, `get_prototype_key` and `build` are removed from the public API, along with `PrototypeBuilder::name`, `now`, `prototype` and `build`, `MomentParser::prototype`, and the `Effect` and `Prototype` types. The simulation sets these itself, so calling them inside `SimulationBuilder::effect` or `prototype` did nothing useful. To migrate, delete those calls: set `now` and `seed` on `SimulationBuilder`, and pick an effect's prototype with `EffectBuilder::prototype_key`.
- **Breaking (Rust API):** `Simulation`, `Proxy` and `Audit` (and their builders) are `Send`, so they can be built on one thread and run on another, including in a spawned async task. Run logs are shared with `Arc` instead of `Rc`, and `SqliteRunLog::new` and `SimpleEventRunLog::new` return the log itself instead of an `Rc`. To migrate, wrap the log where you share it: `.run_log_reader(Arc::new(SimpleEventRunLog::new()))`, or `let log = Arc::new(SqliteRunLog::new(dir)?);` and pass `log.clone()` to the builders. `RunLogReader::last`, `last_for_effect`, `random_for_effect` and `unique_for_effect` return `Option<Arc<Input>>` instead of `Option<Rc<Input>>`. Custom channels, generators and signals (and their builders) must be `Send`; custom formats, parsers and run logs must be `Send + Sync`, so keep shared state in `Arc<Mutex<..>>` instead of `Rc<RefCell<..>>`. The closure given to `ProxyBuilder::on_wait` must be `Send`.
- **Breaking (Rust API):** `SqliteRunLog::new(dir)` is renamed `SqliteRunLog::in_dir(dir)`, since it opens `log.sqlite` inside a directory rather than taking the database file, and it accepts any path (`&str`, `&Path`, `PathBuf`, ...) instead of only a `PathBuf`. To migrate, rename the call and drop any `.to_path_buf()` or `.into()` made to build the `PathBuf`.
- Spec errors point at the exact field they concern. An unknown generator, channel, format or signal `type` is reported at that `type` field and names the type given (`unknown generator type "nope"`, or `missing signal type`). A trigger that isn't a string or an object says what was expected, and an error in a nested generator names its full path (e.g. `effects.a.generator.properties.x.items`, or `generators.<name>.generator` inside a custom type).
- **Breaking (Rust API):** `ParseErrorKind::UnknownType` has a `found` field with the `type` given. `BuildError::Generator` has a `spec_path` field, also readable with `BuildError::spec_path()`. A `SignalParser` returns error paths relative to the signal (`query`, not `signals.<key>.query`), and the dialect adds the `signals.<key>` prefix, as it already does for channels.
- An invalid spec is reported as an error instead of panicking. A `number` generator without the bounds it needs is a parse error, and a trigger expression that stops returning a non-negative number ends the run with an error after logging what ran.
- A `rngo-cli` build with a bumped `zip` fixes RUSTSEC-2026-0546, 0547 and 0548.
- **Breaking (Rust API):** builders are named and shaped alike. `build::string()` returns `StringBuilder` (renamed from `StrBuilder`). `build::sql_format()` returns a `SqlFormatBuilder` instead of the format itself, and the new `build::template_format().template(..)` replaces `TemplateFormat::new(..)`; `SqlFormat` and `TemplateFormat` are no longer public. Formats get a builder trait, `format::FormatBuilder`, which every `Format` implements as its own builder: channel builders' `format()` takes one, and `FormatParser::parse` returns `Box<dyn FormatBuilder>`. `GeneratorBuilder::build` and `ChannelBuilder::build` take `self: Box<Self>` instead of `&self`, like `SignalBuilder::build`, so every `build` consumes its builder. To migrate, rename `StrBuilder` to `StringBuilder`, replace `TemplateFormat::new(t)?` with `build::template_format().template(t)` (an invalid template is now a build error), and in custom generator and channel builders change `&self` to `self: Box<Self>` (move fields out instead of cloning them). A custom `FormatParser` can keep returning its format, boxed as `Box<dyn FormatBuilder>`; a custom channel that takes a format stores the `Box<dyn FormatBuilder>` and calls `build(channel_key)` on it when the channel is built.
- Builder methods that take a string accept `&str` and `String` alike: `Effect::builder`, `EffectBuilder::trigger_effect` and `trigger_expression`, `SimulationBuilder::effect` and `prototype`, and `ObjectBuilder::property`. Callers no longer need `.to_string()`, and existing calls keep compiling, except `.into()` on a literal passed to one of these, which now needs the `.into()` removed.
- **Breaking (Rust API):** `SignalParser::parse` takes only the spec signal (no `key`) and returns a `Box<dyn signal::SignalBuilder>`, which the audit builds with the signal's key, as channel and format parsers already do. Every `Signal` is its own `SignalBuilder`. To migrate a custom signal parser, drop the `key` parameter and change the return type; it can keep returning the signal it parsed. If the signal needs its key, return a builder whose `build(key)` receives it. A `sql` signal's error no longer starts with ``signal `<key>`:``, since the outcome is already reported under its key.
- **Breaking (Rust API):** the public data structs are `#[non_exhaustive]`, so they can gain fields in a minor release: `Input`, `SkippedInput`, `Output`, `Metadata`, `channel::ChannelOutput`, `generator::GeneratorMetadata`, `generator::GeneratorEdge`, `signal::SignalEval` and `AuditReport`. Their fields stay public to read. To migrate, replace struct literals with the constructors, which take the required fields and set the rest with chained methods: `Input::new(id, effect, timestamp, data)` (`.metadata(..)`), `Output::new(channel, level, data)` (timestamped now; `.input_id(..)`, `.timestamp(..)`, `.metadata(..)`), `Metadata::new(kind)` (`.input_id(..)`, `.output_id(..)`, `.data(..)`, `.segment(..)`, `.timestamp(..)`), `ChannelOutput::new(level, message)`, `generator::GeneratorMetadata::new(kind)` (`.attribute(..)`, `.data(..)`), `GeneratorEdge::new(kind, key)` and `SignalEval::new(expectation, passed)`. Patterns that destructure one of them need a trailing `..`.
- **Breaking (Rust API):** `signal::SignalOutcome`'s `Success` and `Error` variants are `#[non_exhaustive]`, so they can gain fields in a minor release. To migrate, build them with the new constructors: replace `SignalOutcome::Success { value, eval }` with `SignalOutcome::success(value, eval)` and `SignalOutcome::Error { error }` with `SignalOutcome::error(error)`. Patterns on either variant need a trailing `..`, e.g. `SignalOutcome::Error { error, .. }`.
- **Breaking (Rust API):** the variants of `BuildError` and `ChannelError` are `#[non_exhaustive]`, so they can gain fields in a minor release. To migrate, build them with the constructors instead of struct literals: `BuildError::simulation(..)`, `effect(..)`, `generator(..)`, `proxy(..)`, `channel(..)`, `signal(..)` and `audit(..)`, and the new `ChannelError::write(channel, source)`, `exited(channel, status, stderr)` and `spawn(channel, source)` (for custom channels). Patterns on a variant need a trailing `..`, e.g. `BuildError::Effect { effect, .. }`.
- **Breaking (Rust API):** every method that a custom signal, channel or format (or its builder or parser) implements takes a context, so it can be given more information in a minor release without breaking your code. What each method got before is on its context. To migrate:
  - `Signal::evaluate(&self, run_log)` is `evaluate(&self, context: &SignalContext)`; use `context.run_log()`.
  - `SignalBuilder::build(self, key)` is `build(self, context: SignalBuildContext)`; use `context.key()`.
  - `SignalParser::parse(&self, signal)` takes a second argument, `context: &SignalParseContext`, whose `spec()` is the whole spec.
  - `Channel::send(&mut self, input)` takes a second argument, `context: &SendContext`. Pass `context.format_context()` to the channel's format.
  - `ChannelBuilder::build(self, channel_key, outputs)` is `build(self, context: ChannelBuildContext)`; use `context.key()` and `context.outputs()`, and build the channel's format with `format.build(context.format_context())`.
  - `ChannelParser::parse(&self, channel, formats: &FormatParseContext)` takes a `&ChannelParseContext` instead. Its `parse(path, value)` is renamed `parse_format`, and it has `spec()`.
  - `Format::format(&self, input)` takes a second argument, `context: &FormatContext`.
  - `FormatBuilder::build(self, channel_key)` is `build(self, context: FormatBuildContext)`; use `context.channel_key()`.
  - `FormatParser::parse(&self, format, spec)` takes `context: &FormatParseContext` instead of the spec; use `context.spec()`.
- **Breaking (Rust API):** `RunLogError::Sqlite` holds a boxed error instead of a `rusqlite::Error`, and `RunLogError` no longer implements `From<rusqlite::Error>`, so upgrading `rusqlite` (for example, to pick up a SQLite security fix) is no longer a breaking change. The message is unchanged. To migrate, match the variant with `RunLogError::Sqlite(..)`, and if you need the `rusqlite` error, downcast its `source()` to `rusqlite::Error`.
- **Breaking (Rust API):** public field names and types are consistent. Spec files and run logs don't change. To migrate:
  - The Rust field for a `type` key is `kind` everywhere: `spec::Format::ftype`, `spec::Generator::stype`, `spec::Channel::ctype`, `spec::Signal::stype`, `Metadata::mtype` and the generator metadata's `mtype` are all renamed `kind`.
  - `generator::Metadata` is renamed `generator::GeneratorMetadata`, so `rngo::Metadata` is only the run log row.
  - `Metadata::input_id` and `output_id` (and their setters) are `Option<u64>` instead of `Option<i64>`, matching `Input::id`. Drop any `as i64` casts.
  - `GeneratorEdge::kind` is a `generator::GeneratorEdgeKind` (`Property`, `Variable`, `Option`, `Items`, or `Other(&'static str)` for a custom generator's own edges) instead of a `&'static str`, and `GeneratorEdge::new` takes one. Replace `GeneratorEdge::new("property", key)` with `GeneratorEdge::new(GeneratorEdgeKind::Property, key)`, and a custom kind `"x"` with `GeneratorEdgeKind::Other("x")`. `GeneratorEdgeKind::as_str()` returns the old string.
  - `spec::Spec` has no `scenarios` field. A top-level `scenarios` key is still rejected with the same error.

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
