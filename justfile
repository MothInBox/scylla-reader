workspace := "scylla-reader"

default: test

# Run cargo check
check:
    cargo check --manifest-path {{workspace}}/Cargo.toml

# Run all tests
test:
    cargo test --manifest-path {{workspace}}/Cargo.toml

# Run clippy
lint:
    cargo clippy --manifest-path {{workspace}}/Cargo.toml -- -D warnings

# Run rustfmt check
fmt:
    cargo fmt --manifest-path {{workspace}}/Cargo.toml --check

# Auto-fix clippy suggestions
fix:
    cargo clippy --manifest-path {{workspace}}/Cargo.toml --fix --allow-dirty -- -D warnings

# Format code
format:
    cargo fmt --manifest-path {{workspace}}/Cargo.toml

# Full CI pipeline: check + test + lint + fmt-check
ci: check test lint fmt

# Watch mode — re-run check on file changes (requires cargo-watch)
watch:
    cargo watch --manifest-path {{workspace}}/Cargo.toml -x check

# Build debug
build:
    cargo build --manifest-path {{workspace}}/Cargo.toml

# Build release
release:
    cargo build --manifest-path {{workspace}}/Cargo.toml --release

# Run the app
run:
    cargo run --manifest-path {{workspace}}/Cargo.toml
