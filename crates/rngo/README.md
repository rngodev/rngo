# rngo

The `rngo` library lets you define and run simulations in Rust code. You can define a simulation with a builder DSL:

```rust
let mut simulation = rngo::Simulation.builder()
    .seed(41)
    .start(TimeDelta.months(-3))
    .end(TimeDelta.zero())
    .with_effect("user", |effect| {
        effect
            .trigger_expression("hz(10, hour) * (offset * 0.0001)")
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
    .build()?;
```

You can also build a simulation from JSON. You'd express the above like this in JSON:

```json
{
    "seed": 41,
    "start": "now - months(3)",
    "end": "now",
    "effects": {
        "user": {
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
                    "created_at": { "type": "context", "path": ["clock", "offset"] }
                }
            }
        },
        "post": {
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
                    "created_at": { "type": "context", "path": ["sim", "offset"] }
                }
            }
        }
    }
}
```

You can parse it like this:

```rust
let value: serde_json::Value = serde_json::from_str(raw).unwrap();
let builder = rngo::Dialect::primitive().parse_simulation_json(value)?;
let mut simulation = simulation.build()?;
```

Both produce a `Simulation` which is an iterator over effects:

```rust
for event in simulation {
    println!("{}", serde_json::to_string(&event).unwrap());
}
```

Which outputs JSON lines like:

```json
{"type":"effect","id":1,"key":"user","offset":0,"value":{"id":1,"name":"Gvtlzqnbhf","age":42,"created_at":0},"format":null}
{"type":"effect","id":2,"key":"post","offset":36,"value":{"id":1,"user_id":1,"title":"Post: Abcdefghijklmno","tags":["a","b","a"],"created_at":36},"format":null}
{"type":"effect","id":3,"key":"user","offset":371,"value":{"id":2,"name":"Rqmzwlxpjt","age":null,"created_at":371},"format":null}
...
```
