.PHONY: all build test clean run dev check lint fmt fmt-check doc

# Default target
all: build

# Build release binary
build:
	cargo build --locked --release

# Build debug binary
dev:
	cargo build --locked

# Run tests
test:
	cargo test --locked

# Run with debug output
run:
	cargo run --locked

# Check code without building
check:
	cargo check --locked

# Lint with clippy
lint:
	cargo clippy --locked -- -D warnings

# Format code
fmt:
	cargo fmt --all

# Check formatting without changing an active worktree
fmt-check:
	cargo fmt --all -- --check

# Generate documentation
doc:
	cargo doc --locked --no-deps --open

# Clean build artifacts
clean:
	cargo clean
	rm -f *.wav *.pcm

# Watch and rebuild on changes (requires cargo-watch)
watch:
	cargo watch -x build

# Install binary to ~/.cargo/bin
install:
	cargo install --locked --path omnivox-cli

# Build the main server and adjacent Piper helper. Native dependencies are
# linked only into the helper (requires cmake + network on first run).
build-piper:
	cargo build --locked --release -p omnivox-piper-helper --features piper
	cargo build --locked --release -p omnivox-cli --features piper

# Install both executables beside one another in ~/.cargo/bin
install-piper:
	cargo install --locked --path omnivox-piper-helper --features piper
	cargo install --locked --path omnivox-cli --features piper
