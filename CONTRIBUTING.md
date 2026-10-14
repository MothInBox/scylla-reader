# Contributing

## Setup

You need Nix with flakes enabled. Everything else lives in the dev shell:

```sh
nix develop
just ci    # check, test, clippy, fmt
```

Common recipes: `just test`, `just lint`, `just fmt`, `just build`, `just run`, `just watch`.

The workspace holds four crates: `scylla-reader` (TUI), `scylla-server` (REST API, storage, jobs, embeddings), `scylla-core` (shared types, worker, messaging), and `scylla-plugin-api` (plugin-facing types). [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) covers how they fit together.

New Rust dependencies belong in both `Cargo.toml` and the dev shell in `flake.nix`; the two files move in lockstep.

## Without Nix

Everything runs on plain cargo once you have a stable Rust toolchain and the system dependencies listed in the README:

```sh
cargo test
cargo clippy -- -D warnings
cargo fmt --all --check
cargo build --release -p scylla-server && cargo run -p scylla-reader
```

The just recipes are thin wrappers around these commands. The pre-commit hooks run cargo fmt and clippy through the dev shell and cover the scylla-reader crate only, so outside Nix, run the three gate commands above yourself before committing. The maintainer develops on Nix and does not test non-Nix setups regularly; the paths above are best-effort.

## Git flow

- `dev` is the integration branch and must stay green. `main` is stable.
- Branch from an up-to-date `dev` with the name `feat/<feature>` or `fix/<topic>`.
- Keep branches short-lived. Merge `dev` into your branch before opening a PR if `dev` moved ahead.
- One coherent change per commit, conventional-commit style: `feat:`, `fix:`, `docs:`, `chore:`.
- The full gate (`cargo test`, `cargo clippy -- -D warnings`, `cargo fmt --all --check`) must pass before you commit; the pre-commit hook enforces the fmt and clippy halves.
- Every user-visible change adds a CHANGELOG entry under `[Unreleased]`.

Phases of one feature land as a sequence of branches, each merged to `dev` before the next starts. Worktrees fit parallel lanes only when the files and interfaces do not overlap.

## Reporting a security issue

Report anything that could compromise a machine running Scylla through [GitHub private vulnerability reporting](https://github.com/MothInBox/scylla-reader/security/advisories/new) instead of a public issue. Note that plugins execute arbitrary WebAssembly; the trust boundary sits with whoever provides the plugin.
