use rngo::build::*;
use rngo::{SimpleEventRunLog, Simulation};

#[test]
fn builders_accept_str_and_string() {
    let simulation = Simulation::builder()
        .prototype("proto", |p| p)
        .prototype(String::from("owned"), |p| p)
        .effect("upstream", |e| {
            e.trigger_expression("hz(1, hour)")
                .generator(object().property("a", constant().value(1)))
        })
        .effect(String::from("downstream"), |e| {
            e.trigger_effect(String::from("upstream"))
                .generator(object().property(String::from("b"), constant().value(1)))
        })
        .run_log_reader(SimpleEventRunLog::new())
        .build();
    assert!(simulation.is_ok());
}
