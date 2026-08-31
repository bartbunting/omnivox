.PHONY: all build test elisp-test clean run dev check lint fmt fmt-check docs-check doc prepare-piper prepare-piper-test-model stage-piper build-piper package-piper verify-piper package-piper-source verify-piper-source install-piper

ELISP_EMACS ?= emacs
PYTHON ?= python3
OMNIVOX_INSTALL_BIN ?= $(HOME)/.cargo/bin

# Default target
all: build

# Build release binary
build:
	$(PYTHON) tools/build.py --release --package omnivox-cli --features piper

# Build debug binary
dev:
	$(PYTHON) tools/build.py --package omnivox-cli --features piper

# Run tests
test:
	cargo test --locked

# Exercise the standalone Emacspeak compatibility adapter without requiring
# an Emacspeak checkout.
elisp-test:
	$(ELISP_EMACS) -Q --batch -l elisp/omnivox-voices-tests.el

# Run with staged debug runtime data
run: dev
	cargo run --locked --package omnivox-cli --features piper

# Check code without building
check:
	cargo check --locked

# Lint with clippy
lint:
	cargo clippy --locked --all-targets -- -D warnings

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
	cargo install --locked --path omnivox-cli --features piper
	mkdir -p "$(OMNIVOX_INSTALL_BIN)/espeak-ng-data" "$(OMNIVOX_INSTALL_BIN)/third-party-licenses"
	cp -R target/release/espeak-ng-data/. "$(OMNIVOX_INSTALL_BIN)/espeak-ng-data/"
	cp -R target/release/third-party-licenses/. "$(OMNIVOX_INSTALL_BIN)/third-party-licenses/"
	cp target/release/LICENSE target/release/LICENSING.md "$(OMNIVOX_INSTALL_BIN)/"

# Build and stage the isolated Piper companion. Its preparation step downloads
# checksum-locked native inputs on first use; repeated builds can run offline.
prepare-piper:
	$(PYTHON) tools/prepare_piper_inputs.py

prepare-piper-test-model:
	$(PYTHON) tools/prepare_piper_test_model.py

stage-piper:
	$(PYTHON) tools/build_piper.py --release

# Build the main server and isolated Piper companion together.
build-piper: stage-piper
	$(PYTHON) tools/build.py --release -p omnivox-cli --features piper

# Create and structurally verify the optional native companion archive.
# Set PIPER_MODEL to add end-to-end synthesis with target/release/omnivox.
package-piper: stage-piper
	$(PYTHON) tools/package_piper.py

verify-piper: package-piper
	$(PYTHON) tools/verify_piper_release.py

# Create and verify the platform-neutral corresponding-source and locked
# build-input artifact for all four native Piper companions.
package-piper-source:
	$(PYTHON) tools/package_piper_source.py

verify-piper-source: package-piper-source
	$(PYTHON) tools/verify_piper_source.py

# Install the server payload and the isolated Piper companion.
install-piper: build-piper
	cargo install --locked --path omnivox-cli --features piper
	mkdir -p "$(OMNIVOX_INSTALL_BIN)/espeak-ng-data" "$(OMNIVOX_INSTALL_BIN)/third-party-licenses" "$(OMNIVOX_INSTALL_BIN)/piper"
	cp -R target/release/espeak-ng-data/. "$(OMNIVOX_INSTALL_BIN)/espeak-ng-data/"
	cp -R target/release/third-party-licenses/. "$(OMNIVOX_INSTALL_BIN)/third-party-licenses/"
	cp -R target/release/piper/. "$(OMNIVOX_INSTALL_BIN)/piper/"
	cp target/release/LICENSE target/release/LICENSING.md "$(OMNIVOX_INSTALL_BIN)/"
