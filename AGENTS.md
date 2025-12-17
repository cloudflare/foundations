# AGENTS.md

## Build & Test Commands
- Build: `cargo build`
- Test all: `cargo test`
- Test single: `cargo test <test_name>` or `cargo test --test <file> <test_name>`
- Clippy: `cargo clippy --all-targets -- -D warnings -D unreachable_pub -D clippy::await_holding_lock -D clippy::clone_on_ref_ptr`
- Format: `cargo fmt --all`
- Lint fix: `./scripts/lint-fix.sh`
- Feature check: `cargo hack check --feature-powerset --no-dev-deps --depth 1`

## Code Style
- Rust 2021 edition, use `rustfmt` defaults
- Imports: group std, external crates, then internal modules; use `crate::` for internal imports
- Types: prefer `Box<dyn Error + Send + Sync>` for generic errors; use `anyhow::Result` for bootstrap errors
- Naming: snake_case for functions/variables, PascalCase for types, SCREAMING_SNAKE for constants
- Errors: use `BootstrapResult<T>` (anyhow) for initialization, `Result<T>` (boxed error) for runtime
- Docs: add `///` doc comments for public items; `#![warn(missing_docs)]` is enabled
- Feature flags: wrap platform/optional code with `#[cfg(feature = "...")]`
- No `openssl`/`openssl-sys` - use `boring`, `ring`, or `rustls` instead
