The `rngo` library lets you build a [spec](https://rngo.dev/docs/spec) in Rust and evaluate it.

A **spec** describes the behavior of a system using the following components:
- a `Simulation` generates a deterministic stream of inputs
- a `Proxy` routes inputs to the appropriate system interfaces and captures outputs
- an `Audit` surfaces pattern in logged inputs and outputs and may set expectations

Each is connected via a log, which persists all inputs, outputs and metadata. `SqliteLog` writes it to `log.sqlite`, whose tables SQL signals query; they are documented in [LOG.md](https://github.com/rngodev/rngo/blob/main/LOG.md).

## DSL

You can define a spec using a builder DSL. Its `start` and `end` take the same expressions as the JSON spec, a `jiff::Timestamp`, or a `jiff::SignedDuration` relative to now:

```rust,no_run
use std::sync::Arc;
use serde_json::Value;
use rngo::build::*;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // The log records inputs, outputs and metadata. `SqliteLog` writes
    // `log.sqlite` into the directory you give it.
    let log = Arc::new(rngo::SqliteLog::in_dir("run")?);

    // The simulation generates a deterministic stream of inputs.
    let mut simulation = rngo::Simulation::builder()
        .seed(41)
        .start("now - months(3)")
        .end("now")
        .prototype("recent", |prototype| {
            prototype
                .start("now - days(7)")
                .end("now")
        })
        .effect("user", |effect| {
            effect
                .channel("db")
                .trigger_expression("hz(10, hour) * (1.0 + offset * 0.0001)")
                .limit(1000)
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
                .channel("db")
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
        .log_reader(log.clone())
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
        .channel("log", stream().command("tail -F logs/app.log"))
        .log_writer(log.clone())
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
        .log(log)
        .build()?;

    // To evaluate the spec, run the simulation against the proxy...
    for step in &mut simulation {
        if let rngo::Step::Input(input) = step? {
            proxy.send(&input)?;
        }
    }

    proxy.finish()?;

    // ...then run the audit against the log.
    let audit_report = audit.run()?;
    assert!(audit_report.passed());

    Ok(())
}
```

`stream` sends each statement on its own, so each one is a round trip and a commit. To load many
backdated inputs, use `batch()` instead: it appends inputs to numbered files under the proxy's
`dir` (a temporary directory if unset) and runs its command once per file (every `size` inputs,
10,000 by default, and once more at `finish`), with the file's path in `RNGO_BATCH_FILE` and its
contents on stdin. A command that exits non-zero fails the run, and its file is kept. With `psql`,
`-1` makes each batch one transaction, and `ON_ERROR_STOP` is needed for it to fail at all:
without it, `psql` exits 0 after a failed statement and the batch rolls back silently.

```text
psql -X -q -1 -v ON_ERROR_STOP=1 -f "$RNGO_BATCH_FILE" "$DATABASE_URL"
```

Even batched, `INSERT`s cost one round trip per row. Bulk loaders such as Postgres `COPY`, MySQL
`LOAD DATA LOCAL INFILE`, SQLite `.import` and DuckDB `read_csv` read a whole file at once, but
expect one CSV file per table. For them, use `csv_format()`, which writes an input's `data`
object as one CSV row with no header (one field per key, in key order; `null` as an empty field,
strings always quoted, arrays and objects as quoted JSON), and `split_by_effect()` on the batch.
Each batch then writes one file per effect and column list (`size` still counts the whole batch)
and runs the command once per file, with three more variables:

- `RNGO_BATCH_EFFECT`: the effect key.
- `RNGO_BATCH_TABLE`: the effect's `metadata.table` in a spec (`effect_table` in Rust), else the
  effect key.
- `RNGO_BATCH_COLUMNS`: the file's columns, comma-separated and unquoted.

```text
psql -X -v ON_ERROR_STOP=1 "$DATABASE_URL" -c "\copy $RNGO_BATCH_TABLE ($RNGO_BATCH_COLUMNS) FROM '$RNGO_BATCH_FILE' WITH (FORMAT csv)"
```

On Neon, use the direct (non-pooled) connection string for bulk loads. Postgres folds unquoted
names to lowercase, so for mixed-case tables or columns quote them in the command yourself.

Files load parents first: an effect's files load after those of every effect it references
through a `reference` generator or an effect trigger, so foreign keys hold (in Rust, or for
references a custom Rust generator makes, declare them with `load_after`). A batch split by
effect isn't one transaction: if a file fails, the run stops with an error naming its effect and
table, files already loaded stay loaded and are deleted, and the failed file and those after it
are kept. For effects that reference each other in a cycle, make the foreign keys `DEFERRABLE
INITIALLY DEFERRED` and load the cycle's files in one transaction yourself, or drop the
constraints for the load.

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
use std::sync::Arc;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let raw = std::fs::read_to_string("spec.json")?;
    let value: serde_json::Value = serde_json::from_str(&raw)?;
    let spec = rngo::spec::from_value(value)?;
    let dialect = rngo::Dialect::primitive();
    let log = Arc::new(rngo::SqliteLog::in_dir("run")?);

    let mut simulation = dialect
        .parse_simulation(&spec)?
        .log_reader(log.clone())
        .build()?;

    let mut proxy = dialect
        .parse_proxy(&spec)?
        .log_writer(log.clone())
        .build()?;

    let audit = dialect
        .parse_audit(&spec)?
        .log(log)
        .build()?;

    // Run the simulation against the proxy, then audit the log.
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
