default: test

# Run cargo check
check:
    cargo check --workspace

# Run all tests
test:
    cargo test --workspace

# Run clippy
lint:
    cargo clippy --workspace -- -D warnings

# Run rustfmt check
fmt:
    cargo fmt --all --check

# Auto-fix clippy suggestions
fix:
    cargo clippy --workspace --fix --allow-dirty -- -D warnings

# Format code
format:
    cargo fmt --all

# Full CI pipeline: check + test + lint + fmt-check
ci: check test lint fmt

# Watch mode — re-run check on file changes (requires cargo-watch)
watch:
    cargo watch -x check

# Build debug (reader + server)
build:
    cargo build -p scylla-reader -p scylla-server

# Build release (reader + server)
release:
    cargo build --release -p scylla-reader -p scylla-server

# Run the app (builds the server first so the TUI can spawn it)
run:
    cargo build -p scylla-server && cargo run -p scylla-reader
