mod cms 'examples/cms/justfile'

fmt:
    cargo fmt --all

clippy:
    cargo clippy --fix --workspace --all-targets -- -D warnings
