The `rngo` library lets you build a [spec](https://rngo.dev/docs/spec) in Rust and evaluate it.

A **spec** describes the behavior of a system using the following components:
- a `Simulation` generates a deterministic stream of inputs
- a `Proxy` routes inputs to the appropriate system interfaces and captures outputs
- an `Audit` surfaces pattern in logged inputs and outputs and may set expectations

Each is connected via a `Log` instance, which persists all input, outputs and metadata.

## DSL

You can define a spec using a builder DSL:

```rust,no_run
use std::num::NonZeroU64;
use jiff::SignedDuration;
use serde_json::Value;
use rngo::build::*;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // The run log records inputs, outputs and metadata. `SqliteRunLog` writes
    // `log.sqlite` into the directory you give it.
    let run_log = rngo::SqliteRunLog::new("run".into())?;

    // The simulation generates a deterministic stream of inputs.
    let mut simulation = rngo::Simulation::builder()
        .seed(41)
        .start(Moment::Relative(SignedDuration::from_hours(-2160)))
        .end(Moment::Relative(SignedDuration::ZERO))
        .prototype("recent", |prototype| {
            prototype
                .start(Moment::Relative(SignedDuration::from_hours(-168)))
                .end(Moment::Relative(SignedDuration::ZERO))
        })
        .effect("user", |effect| {
            effect
                .trigger_expression("hz(10, hour) * (1.0 + offset * 0.0001)")
                .limit(NonZeroU64::new(1000).unwrap())
                .generator(
                    object()
                        .property("id", number().minimum(1).scale(0).step(1))
                        .property("name", string().pattern(".{10,50}"))
                        .property(
                            "age",
                            select()
                                .option(3, number().minimum(18).maximum(65))
                                .option(1, constant().value(Value::Null)),
                        )
                        .property("created_at", context().path(["clock", "now"])),
                )
        })
        .effect("post", |effect| {
            effect
                .prototype_key("recent")
                .trigger_expression("hz(100, hour) * (1.0 + offset * 0.0001)")
                .generator(
                    object()
                        .property("id", number().minimum(1).scale(0).step(1))
                        .property(
                            "user_id",
                            function()
                                .expression("user.id")
                                .variable("user", reference().effect("user")),
                        )
                        .property("title", string().pattern("Post: .{10,20}"))
                        .property(
                            "tags",
                            array().min_items(0).max_items(10).items(
                                select()
                                    .option(1, constant().value("a"))
                                    .option(1, constant().value("b")),
                            ),
                        )
                        .property("created_at", context().path(["clock", "now"])),
                )
        })
        .run_log_reader(run_log.clone())
        .build()?;

    // The proxy routes inputs to the system's interfaces and captures outputs.
    let mut proxy = rngo::Proxy::builder()
        .channel(
            "db",
            stream()
                .command("psql -q $DATABASE_URL")
                .format(
                    sql_format()
                        .effect_table("user", "USERS")
                        .effect_table("post", "POSTS")
                )
        )
        .effect_channels([("user", "db"), ("post", "db")])
        .channel("log", stream().command("tail -F logs/app.log"))
        .run_log_writer(run_log.clone())
        .build()?;

    // The audit checks patterns in the logged inputs and outputs.
    let audit = rngo::Audit::builder()
        .signal("some-inputs",
            sql_signal()
                .query("SELECT count(*) FROM inputs;")
                .expect("result > 0")
        )
        .signal("no-psql-errors",
            sql_signal()
                .query("SELECT count(*) FROM outputs WHERE channel = 'db'")
                .expect("result == 0")
        )
        .signal("minimal-log-errors",
            sql_signal()
                .query("SELECT count(*) FROM outputs WHERE channel = 'log' AND data LIKE 'ERROR%'")
                .expect("result < 20")
        )
        .run_log(run_log)
        .build()?;

    // To evaluate the spec, run the simulation against the proxy...
    for step in &mut simulation {
        if let rngo::Step::Input(input) = step? {
            proxy.send(&input)?;
        }
    }

    proxy.finish()?;

    // ...then run the audit against the run log.
    let audit_report = audit.run()?;
    assert!(audit_report.passed());

    Ok(())
}
```

## Spec

You can also define the above as a JSON (or YAML) spec:

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
            "trigger": "hz(10, hour) * (1.0 + offset * 0.0001)",
            "limit": 1000,
            "generator": {
                "type": "object",
                "properties": {
                    "id": { "type": "number", "minimum": 1, "scale": 0, "step": 1 },
                    "name": { "type": "string", "pattern": ".{10,50}" },
                    "age": {
                        "type": "select",
                        "options": [
                            { "weight": 3, "generator": { "type": "number", "minimum": 18, "maximum": 65 } },
                            { "weight": 1, "generator": { "type": "constant", "value": null } }
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
            "trigger": "hz(100, hour) * (1.0 + offset * 0.0001)",
            "generator": {
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
                                { "weight": 1, "generator": { "type": "constant", "value": "a" } },
                                { "weight": 1, "generator": { "type": "constant", "value": "b" } }
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

Then parse the simulation, proxy and audit and run them:

```rust,no_run
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let raw = std::fs::read_to_string("spec.json")?;
    let value: serde_json::Value = serde_json::from_str(&raw)?;
    let spec = rngo::spec::from_value(value)?;
    let dialect = rngo::Dialect::primitive();
    let run_log = rngo::SqliteRunLog::new("run".into())?;

    let mut simulation = dialect
        .parse_simulation(&spec)?
        .run_log_reader(run_log.clone())
        .build()?;

    let mut proxy = dialect
        .parse_proxy(&spec)?
        .effect_channels(simulation.effect_channels())
        .run_log_writer(run_log.clone())
        .build()?;

    let audit = dialect
        .parse_audit(&spec)?
        .run_log(run_log)
        .build()?;

    // Run the simulation against the proxy, then audit the run log.
    for step in &mut simulation {
        if let rngo::Step::Input(input) = step? {
            proxy.send(&input)?;
        }
    }

    proxy.finish()?;

    let audit_report = audit.run()?;
    assert!(audit_report.passed());

    Ok(())
}
```
