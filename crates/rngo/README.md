# rngo

The `rngo` library lets you evaluate a **spec** in Rust.

A **spec** describes the behavior of a system using the following components:
- a `RunLog` records inputs, outputs and metadata
- a `Simulation` generates a deterministic stream of inputs
- a `Proxy` routes inputs to the appropriate system interfaces and captures outputs
- an `Audit` surfaces pattern in logged inputs and outputs and may set expectations

## Public API

Specs are defined as JSON or YAML and parsed with `spec::from_value`. These items are the supported public API; everything else is private and may change in any release:

- `spec`: `Spec` and `from_value`
- `Dialect`: parses a `Spec` into the builders below
- `SimulationBuilder`, `Simulation`, `StandaloneSimulation`, `ProxyBuilder`, `Proxy`, `StopHandle`, `AuditBuilder`, `Audit`, `AuditReport`
- `SqliteRunLog`, `SimpleEventRunLog`, `RunLogReader`, `RunLogWriter`
- `Input`, `SkippedInput`, `Step`, `Metadata`, `Output`, `Level`, `SignalOutcome`
- `error` types: `ParseError`, `ParseErrorKind`, `BuildError`, `RunError`, `RunLogError`, `FormatError`, `ChannelError`, `ProxyError`, `SignalError`, `EffectKey`, `SimulationKey`

Public enums and spec structs are `#[non_exhaustive]`, so they can gain variants and fields without a breaking change.

## Spec

A spec looks like this:

```json
{
    "seed": 41,
    "start": "now - months(3)",
    "end": "now",
    "prototypes": {
        "recent": {
            "start": "now - days(7)",
            "end": "now",
            "scenarios": { "backfill": { "start": "now - months(6)" } }
        }
    },
    "effects": {
        "user": {
            "channel": "db",
            "metadata": { "table": "USERS" },
            "trigger": "hz(10, hour) * (offset * 0.0001)",
            "limit": 1000,
            "schema": {
                "type": "object",
                "properties": {
                    "id": { "type": "number", "minimum": 1, "scale": 0, "step": 1 },
                    "name": { "type": "string", "pattern": ".{10,50}" },
                    "age": {
                        "type": "select",
                        "options": [
                            { "weight": 3, "schema": { "type": "number", "minimum": 18, "maximum": 65 } },
                            { "weight": 1, "schema": { "type": "constant", "value": null } }
                        ]
                    },
                    "created_at": { "type": "context", "path": ["clock", "now"] }
                }
            }
        },
        "post": {
            "prototype": "recent",
            "channel": "db",
            "metadata": { "table": "POSTS" },
            "trigger": "hz(100, hour) * (offset * 0.0001)",
            "schema": {
                "type": "object",
                "properties": {
                    "id": { "type": "number", "minimum": 1, "scale": 0, "step": 1 },
                    "user_id": {
                        "type": "function",
                        "expression": "user.id",
                        "variables": {
                            "user": { "type": "reference", "effect": "user" }
                        }
                    },
                    "title": { "type": "string", "pattern": "Post: .{10,20}" },
                    "tags": {
                        "type": "array",
                        "minItems": 0,
                        "maxItems": 10,
                        "items": {
                            "type": "select",
                            "options": [
                                { "weight": 1, "schema": { "type": "constant", "value": "a" } },
                                { "weight": 1, "schema": { "type": "constant", "value": "b" } }
                            ]
                        }
                    },
                    "created_at": { "type": "context", "path": ["clock", "now"] }
                }
            }
        }
    },
    "channels": {
        "db": {
            "type": "stream",
            "command": "psql -q $DATABASE_URL",
            "format": { "type": "sql" }
        },
        "log": {
            "type": "stream",
            "command": "tail -F logs/app.log"
        }
    },
    "signals": {
        "some-inputs": {
            "type": "sql",
            "query": "SELECT count(*) FROM inputs;",
            "expect": "result > 0"
        },
        "no-psql-errors": {
            "type": "sql",
            "query": "SELECT count(*) FROM outputs WHERE channel = 'db'",
            "expect": "result == 0"
        },
        "minimal-log-errors": {
            "type": "sql",
            "query": "SELECT count(*) FROM outputs WHERE channel = 'log' AND data LIKE 'ERROR%'",
            "expect": "result < 20"
        }
    }
}
```

You can parse and run like this:

```rust
let value: serde_json::Value = serde_json::from_str(raw).unwrap();
let spec = rngo::spec::from_value(value)?;
let dialect = rngo::Dialect::primitive();
let run_log = rngo::SqliteRunLog::new(".".into())?;

let mut simulation = dialect
    .parse_simulation(spec.clone())?
    .run_log(run_log.clone())
    .build()?;

let mut proxy = dialect
    .parse_proxy(spec.clone())?
    .effect_channels(simulation.effect_channels())
    .run_log_writer(run_log.clone())
    .build()?;

let audit = dialect
    .parse_audit(spec)?
    .run_log(run_log)
    .build()?;

for input in &mut simulation {
    proxy.send(&input)?;
}

proxy.finish()?;

let audit_report = audit.run()?;
assert!(audit_report.passed())
```
