mod cms 'examples/cms/justfile'

fmt:
    cargo fmt --all

clippy:
    cargo clippy --workspace --all-targets -- -D warnings

clippy-fix:
    cargo clippy --fix --workspace --all-targets -- -D warnings
