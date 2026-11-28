use rngo::generator::{GeneratorEdge, GeneratorEdgeKind};
use rngo::{BatchTarget, BuildError, ChannelError, EffectKey, SimulationKey};
use std::io::ErrorKind;

#[test]
fn build_errors_built_outside_the_crate_can_be_read_back() {
    match BuildError::simulation(SimulationKey::Limit, "limit must be positive") {
        BuildError::Simulation {
            key: SimulationKey::Limit,
            message,
            ..
        } => assert_eq!(message, "limit must be positive"),
        other => panic!("expected a simulation error, got {other:?}"),
    }

    match BuildError::effect("user", EffectKey::Trigger, "bad trigger") {
        BuildError::Effect {
            effect,
            key: EffectKey::Trigger,
            ..
        } => assert_eq!(effect, "user"),
        other => panic!("expected an effect error, got {other:?}"),
    }

    let generator = BuildError::generator(
        "user",
        vec![GeneratorEdge::new(GeneratorEdgeKind::Property, "age")],
        "minimum is greater than maximum",
    );
    assert!(matches!(&generator, BuildError::Generator { effect, .. } if effect == "user"));
    assert_eq!(generator.message(), "minimum is greater than maximum");
    assert_eq!(generator.generator_path().unwrap()[0].key, "age");
    assert_eq!(
        generator.spec_path().unwrap(),
        ["effects", "user", "generator", "properties", "age"]
    );

    assert!(matches!(BuildError::proxy("p"), BuildError::Proxy { .. }));
    assert!(
        matches!(BuildError::channel("db", "c"), BuildError::Channel { channel, .. } if channel == "db")
    );
    assert!(
        matches!(BuildError::signal("ok", "s"), BuildError::Signal { signal, .. } if signal == "ok")
    );
    assert!(matches!(BuildError::audit("a"), BuildError::Audit { .. }));
    assert_eq!(BuildError::audit("a").message(), "a");
    assert_eq!(BuildError::proxy("p").spec_path(), None);
}

#[test]
fn channel_errors_built_outside_the_crate_can_be_read_back() {
    match ChannelError::write("db", ErrorKind::BrokenPipe.into()) {
        ChannelError::Write {
            channel, source, ..
        } => {
            assert_eq!(channel, "db");
            assert_eq!(source.kind(), ErrorKind::BrokenPipe);
        }
        other => panic!("expected a write error, got {other:?}"),
    }

    let exited = ChannelError::exited("db", "exit status: 1", "boom");
    assert!(
        matches!(&exited, ChannelError::Exited { status, stderr, .. } if status == "exit status: 1" && stderr == "boom")
    );
    assert_eq!(
        exited.to_string(),
        "channel `db`: command exited (exit status: 1):\nboom"
    );

    assert!(matches!(
        ChannelError::spawn("db", ErrorKind::NotFound.into()),
        ChannelError::Spawn { .. }
    ));

    let batch = ChannelError::batch(
        "db",
        "exit status: 3",
        2,
        "ERROR: relation \"users\" does not exist",
        "batches/db/00000001",
    );
    assert!(
        matches!(&batch, ChannelError::Batch { inputs: 2, path, .. } if path.ends_with("00000001"))
    );
    assert_eq!(
        batch.to_string(),
        "channel `db`: command exited (exit status: 3) loading a batch of 2 inputs, kept at \
         batches/db/00000001:\nERROR: relation \"users\" does not exist"
    );

    let split = ChannelError::batch("db", "exit status: 1", 1, "", "batches/db/00000001/user")
        .with_batch_target(BatchTarget::new("user", "users"));
    assert!(matches!(
        &split,
        ChannelError::Batch { target: Some(target), .. }
            if target.effect == "user" && target.table == "users"
    ));
    assert_eq!(
        split.to_string(),
        "channel `db`: command exited (exit status: 1) loading a batch of 1 input of effect \
         `user` into table `users`, kept at batches/db/00000001/user"
    );
}
