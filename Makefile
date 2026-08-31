.PHONY: all build test elisp-test clean run dev check lint fmt fmt-check docs-check doc stage-piper build-piper install-piper

ELISP_EMACS ?= emacs
PYTHON ?= python3
OMNIVOX_INSTALL_BIN ?= $(HOME)/.cargo/bin

# Default target
all: build

# Build release binary
build:
	$(PYTHON) tools/build.py --release

# Build debug binary
dev:
	$(PYTHON) tools/build.py

# Run tests
test:
	cargo test --locked

# Exercise the standalone Emacspeak compatibility adapter without requiring
# an Emacspeak checkout.
elisp-test:
	$(ELISP_EMACS) -Q --batch -l elisp/omnivox-voices-tests.el

# Run with staged debug runtime data
run: dev
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

# Verify repository-local links in tracked Markdown documentation
docs-check:
	$(PYTHON) tools/check_markdown_links.py

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

# Install the binary and its eSpeak runtime payload to ~/.cargo/bin. Override
# OMNIVOX_INSTALL_BIN when Cargo is configured with a different install root.
install: build
	cargo install --locked --path omnivox-cli
	mkdir -p "$(OMNIVOX_INSTALL_BIN)/espeak-ng-data" "$(OMNIVOX_INSTALL_BIN)/third-party-licenses"
	cp -R target/release/espeak-ng-data/. "$(OMNIVOX_INSTALL_BIN)/espeak-ng-data/"
	cp -R target/release/third-party-licenses/. "$(OMNIVOX_INSTALL_BIN)/third-party-licenses/"
	cp target/release/LICENSE target/release/LICENSING.md "$(OMNIVOX_INSTALL_BIN)/"

# Build and stage the isolated Piper companion (requires cmake + network on
# first run until dependency preparation is implemented).
stage-piper:
	$(PYTHON) tools/build_piper.py --release

# Build the main server and isolated Piper companion together.
build-piper: stage-piper
	$(PYTHON) tools/build.py --release -p omnivox-cli --features piper

# Install the server payload and the isolated Piper companion.
install-piper: build-piper
	cargo install --locked --path omnivox-cli --features piper
	mkdir -p "$(OMNIVOX_INSTALL_BIN)/espeak-ng-data" "$(OMNIVOX_INSTALL_BIN)/third-party-licenses" "$(OMNIVOX_INSTALL_BIN)/piper"
	cp -R target/release/espeak-ng-data/. "$(OMNIVOX_INSTALL_BIN)/espeak-ng-data/"
	cp -R target/release/third-party-licenses/. "$(OMNIVOX_INSTALL_BIN)/third-party-licenses/"
	cp -R target/release/piper/. "$(OMNIVOX_INSTALL_BIN)/piper/"
	cp target/release/LICENSE target/release/LICENSING.md "$(OMNIVOX_INSTALL_BIN)/"
