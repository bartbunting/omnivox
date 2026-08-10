.PHONY: all build test clean run dev check lint fmt doc

# Default target
all: build

# Build release binary
build:
	cargo build --release

# Build debug binary
dev:
	cargo build

# Run tests
test:
	cargo test

# Run with debug output
run:
	cargo run

# Check code without building
check:
	cargo check

# Lint with clippy
lint:
	cargo clippy -- -D warnings

# Format code
fmt:
	cargo fmt

# Generate documentation
doc:
	cargo doc --no-deps --open

# Clean build artifacts
clean:
	cargo clean
	rm -f *.wav *.pcm

# Watch and rebuild on changes (requires cargo-watch)
watch:
	cargo watch -x build

# Install binary to ~/.cargo/bin
install:
	cargo install --path omnivox-cli

# Build the main server and adjacent Piper helper. Native dependencies are
# linked only into the helper (requires cmake + network on first run).
build-piper:
	cargo build --release -p omnivox-piper-helper --features piper
	cargo build --release -p omnivox-cli --features piper

# Install both executables beside one another in ~/.cargo/bin
install-piper:
	cargo install --path omnivox-piper-helper --features piper
	cargo install --path omnivox-cli --features piper
