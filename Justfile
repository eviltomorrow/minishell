default: check

# Build all crates (debug)
build:
    cargo build

# Build in release mode
release:
    cargo build --release

# Fast compile check
check:
    cargo check

# Run all tests
test:
    cargo test

# Run clippy (deny warnings)
lint:
    cargo clippy --all-targets -- -D warnings

# Format code
fmt:
    cargo fmt

# Check formatting without modifying
fmt-check:
    cargo fmt --check

# Auto-fix clippy warnings
fix:
    cargo clippy --fix --all-targets --allow-dirty --allow-staged

# Build documentation (no deps)
docs:
    cargo doc --no-deps

# Clean build artifacts
clean:
    cargo clean

# CI-equivalent: run all quality checks
ci: check test lint fmt-check

# Deploy release binaries to /home/shepard/Applications/minishell
deploy:
    cargo build --release
    cp target/release/minishell /home/shepard/Applications/minishell/minishell
    cp target/release/minishell-server /home/shepard/Applications/minishell/minishell-server
