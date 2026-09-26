# rngo

The `rngo` library lets you assemble **cells** and define **audits** in Rust.

A **cell** is responsible for sending inputs to and capturing outputs from the system under test (SUT). Its components include:
- a `RunLog` that records inputs, outputs and metadata (and may be shared by other cells)
- one or more `Simulation`s that generate and log inputs
- a single `Proxy` that routes the inputs and logs the outputs

An **audit** surfaces patterns in the `RunLog` and usually sets expectations of those patterns.

## DSL

You can define a cell using a builder DSL. First we'll define a `SqliteRunLog`:

```rust
let run_log = rngo::SqliteRunLog::new(".")
```

Next a `Simulation`:

```rust
let mut simulation = rngo::Simulation.builder()
    .seed(41)
    .start(TimeDelta.months(-3))
    .end(TimeDelta.zero())
    .with_effect("user", |effect| {
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
    .with_effect("post", |effect| {
        effect
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
    .run_log(run_log.clone())
    .build()?;
```

And then the `Proxy`:

```rust
let proxy = rngo::Proxy::builder()
    .with_channel("db", |channel| {
        channel
            .effects("user", "post")
            .format(
                sql_format()
                    .effect_table("user", "USERS")
                    .effect_table("post", "POSTS")
            )
            .target(
                stream().command("psql -q $DATABASE_URL")
            )
            
    })
    .with_channel("log", |channel| {
        channel
            .target(
                stream().command("tail -F logs/app.log")
            )
            
    })
    .run_log(run_log.clone())
    .build()?
```

Now we can run the `Simulation` against the `Proxy` (and exit the sub-shells): 

```rust
for input in &mut simulation {
    proxy.send(&input)?;
}

proxy.finish();
```

Finally, we can build an `Audit` and set some expectations for the inputs and ouputs

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

let audit_report = audit.run();
assert!(audit_report.passed())
```

## Spec

You can also define the above in JSON (or YAML) spec - it would look like this:

```json
{
    "seed": 41,
    "start": "now - months(3)",
    "end": "now",
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
            "format": { "type": "sql" },
            "target": { "type": "stream", "command": "psql -q $DATABASE_URL" }
        },
        "log": {
            "target": { "type": "stream", "command": "tail -F logs/app.log" }
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
let run_log = rngo::SqliteRunLog::new(".")

let mut simulation = dialect
    .parse_simulation(spec.clone())?
    .run_log(run_log.clone())
    .build()?;

let mut proxy = dialect
    .parse_proxy(spec.clone())?
    .run_log_writer(run_log.clone())
    .build()?;

let audit = dialect
    .parse_audit(spec)?
    .run_log(run_log)
    .build()?;

for input in &mut simulation {
    proxy.send(&input)?;
}

proxy.finish();

let audit_report = audit.run();
assert!(audit_report.passed())
```
