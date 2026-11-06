# rngo

The `rngo` library lets you build a **spec** in Rust and evaluate it.

A **spec** describes the behavior of a system using the following components:
- a `RunLog` records inputs, outputs and metadata
- a `Simulation` generates a deterministic stream of inputs
- a `Proxy` routes inputs to the appropriate system interfaces and captures outputs
- an `Audit` surfaces pattern in logged inputs and outputs and may set expectations

## DSL

You can define a spec using a builder DSL. First we'll define a `SqliteRunLog`:

```rust
let run_log = rngo::SqliteRunLog::new(".".into())?;
```

Next a `Simulation`:

```rust
let mut simulation = rngo::Simulation::builder()
    .seed(41)
    .start(Moment::Relative(TimeDelta::days(-90)))
    .end(Moment::Relative(TimeDelta::zero()))
    .prototype("recent", |prototype| {
        prototype
            .start(Moment::Relative(TimeDelta::days(-7)))
            .end(Moment::Relative(TimeDelta::zero()))
    })
    .effect("user", |effect| {
        effect
            .trigger_expression("hz(10, hour) * (offset * 0.0001)")
            .limit(NonZeroU64::new(1000).unwrap())
            .schema(
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
            .trigger_expression("hz(100, hour) * (offset * 0.0001)")
            .schema(
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
```

And then the `Proxy`:

```rust
let proxy = rngo::Proxy::builder()
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
    .run_log(run_log.clone())
    .build()?
```

To evaluate the spec, first run the `Simulation` against the `Proxy`: 

```rust
for input in &mut simulation {
    proxy.send(&input)?;
}

proxy.finish()?;
```

Then build an `Audit` and run it against the `RunLog`:

```rust
let audit = rngo::Audit::builder()
    .with_signal("some-inputs",
        sql_signal()
            .query("SELECT count(*) FROM inputs;")
            .expect("result > 0")
    )
    .with_signal("no-psql-errors",
        sql_signal()
            .query("SELECT count(*) FROM outputs WHERE channel = 'db'")
            .expect("result == 0")
    )
    .with_signal("minimal-log-errors",
        sql_signal()
            .query("SELECT count(*) FROM outputs WHERE channel = 'log' AND data LIKE 'ERROR%'")
            .expect("result < 20")
    )
    .run_log(run_log)
    .build()?;

let audit_report = audit.run()?;
assert!(audit_report.passed())
```

## Spec

You can also define the above in JSON (or YAML) spec - it would look like this:

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
