.PHONY: all build test elisp-test latency-benchmark-test latency-benchmark-suite-test server-stress-test helper-soak-test windows-helpers windows-helpers-test windows-helpers-startup-test clean-windows-helpers clean run dev check lint fmt fmt-check docs-check doc stage-rhvoice stage-rhvoice-dev build-rhvoice install-rhvoice prepare-flite stage-flite stage-flite-dev build-flite package-flite verify-flite package-flite-source verify-flite-source install-flite prepare-rutts stage-rutts stage-rutts-dev build-rutts package-rutts verify-rutts package-rutts-source verify-rutts-source install-rutts prepare-piper prepare-piper-test-model stage-piper build-piper package-piper verify-piper package-piper-source verify-piper-source install-piper

ELISP_EMACS ?= emacs
PYTHON ?= python3
OMNIVOX_INSTALL_BIN ?= $(HOME)/.cargo/bin

# Default target
all: build

# Build release binary
build: stage-rhvoice stage-flite stage-rutts
	$(PYTHON) tools/build.py --release --package omnivox-cli --features piper

# Build debug binary
dev: stage-rhvoice-dev stage-flite-dev stage-rutts-dev
	$(PYTHON) tools/build.py --package omnivox-cli --features piper

# Run tests
test: windows-helpers-test latency-benchmark-test latency-benchmark-suite-test server-stress-test helper-soak-test
	cargo test --locked

latency-benchmark-test:
	PYTHONDONTWRITEBYTECODE=1 \
		$(PYTHON) -W error::ResourceWarning tools/test_benchmark_server.py

latency-benchmark-suite-test:
	PYTHONDONTWRITEBYTECODE=1 \
		$(PYTHON) -W error::ResourceWarning tools/test_benchmark_suite.py

server-stress-test:
	PYTHONDONTWRITEBYTECODE=1 \
		$(PYTHON) -W error::ResourceWarning tools/test_stress_server.py

helper-soak-test:
	PYTHONDONTWRITEBYTECODE=1 \
		$(PYTHON) -W error::ResourceWarning tools/test_stress_helper.py

# Build the GPL-2.0-or-later 32-bit Windows capture helpers.  A reproducible
# Emacsvox bundle supplies its pinned compiler and reference assemblies through
# OMNIVOX_CSC and OMNIVOX_REFERENCE_DIR.
windows-helpers:
	$(MAKE) -C windows-helpers

windows-helpers-test:
	$(PYTHON) tools/test_windows_helpers.py

# Build both helpers and verify that absent proprietary runtimes are reported
# through the helper protocol rather than by terminating during process load.
windows-helpers-startup-test: windows-helpers
	$(PYTHON) tools/test_windows_helper_startup.py

clean-windows-helpers:
	$(MAKE) -C windows-helpers clean

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
	mkdir -p "$(OMNIVOX_INSTALL_BIN)/espeak-ng-data" "$(OMNIVOX_INSTALL_BIN)/third-party-licenses" "$(OMNIVOX_INSTALL_BIN)/rhvoice" "$(OMNIVOX_INSTALL_BIN)/flite" "$(OMNIVOX_INSTALL_BIN)/rutts"
	cp -R target/release/espeak-ng-data/. "$(OMNIVOX_INSTALL_BIN)/espeak-ng-data/"
	cp -R target/release/third-party-licenses/. "$(OMNIVOX_INSTALL_BIN)/third-party-licenses/"
	cp -R target/release/rhvoice/. "$(OMNIVOX_INSTALL_BIN)/rhvoice/"
	cp -R target/release/flite/. "$(OMNIVOX_INSTALL_BIN)/flite/"
	cp -R target/release/rutts/. "$(OMNIVOX_INSTALL_BIN)/rutts/"
	cp target/release/LICENSE target/release/LICENSING.md "$(OMNIVOX_INSTALL_BIN)/"

# Build the portable RHVoice helper. The user supplies the separately licensed
# native runtime, language data, and voice data at run time.
stage-rhvoice:
	$(PYTHON) tools/build_rhvoice.py --release

stage-rhvoice-dev:
	$(PYTHON) tools/build_rhvoice.py

build-rhvoice: build

install-rhvoice: install

# Download and verify the exact Flite v2.2 corresponding source used by the
# portable companion build. The native build itself never accesses the network.
prepare-flite:
	$(PYTHON) tools/prepare_flite_inputs.py

stage-flite:
	$(PYTHON) tools/build_flite.py --release

stage-flite-dev:
	$(PYTHON) tools/build_flite.py

build-flite: stage-flite

package-flite: stage-flite
	$(PYTHON) tools/package_flite.py

verify-flite: package-flite
	$(PYTHON) tools/verify_flite_release.py

package-flite-source:
	$(PYTHON) tools/package_flite_source.py

verify-flite-source: package-flite-source
	$(PYTHON) tools/verify_flite_source.py

install-flite: install

# Download and verify the exact RuTTS v6.3.3 source used by the portable
# dictionary-free companion. The native build itself never accesses the network.
prepare-rutts:
	$(PYTHON) tools/prepare_rutts_inputs.py

stage-rutts:
	$(PYTHON) tools/build_rutts.py --release

stage-rutts-dev:
	$(PYTHON) tools/build_rutts.py

build-rutts: stage-rutts

package-rutts: stage-rutts
	$(PYTHON) tools/package_rutts.py

verify-rutts: package-rutts
	$(PYTHON) tools/verify_rutts_release.py

package-rutts-source:
	$(PYTHON) tools/package_rutts_source.py

verify-rutts-source: package-rutts-source
	$(PYTHON) tools/verify_rutts_source.py

install-rutts: install

# Build and stage the isolated Piper companion. Its preparation step downloads
# checksum-locked native inputs on first use; repeated builds can run offline.
prepare-piper:
	$(PYTHON) tools/prepare_piper_inputs.py

prepare-piper-test-model:
	$(PYTHON) tools/prepare_piper_test_model.py

stage-piper:
	$(PYTHON) tools/build_piper.py --release

# Build the main server and isolated Piper companion together.
build-piper: stage-piper stage-rhvoice
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
	mkdir -p "$(OMNIVOX_INSTALL_BIN)/espeak-ng-data" "$(OMNIVOX_INSTALL_BIN)/third-party-licenses" "$(OMNIVOX_INSTALL_BIN)/rhvoice" "$(OMNIVOX_INSTALL_BIN)/piper"
	cp -R target/release/espeak-ng-data/. "$(OMNIVOX_INSTALL_BIN)/espeak-ng-data/"
	cp -R target/release/third-party-licenses/. "$(OMNIVOX_INSTALL_BIN)/third-party-licenses/"
	cp -R target/release/rhvoice/. "$(OMNIVOX_INSTALL_BIN)/rhvoice/"
	cp -R target/release/piper/. "$(OMNIVOX_INSTALL_BIN)/piper/"
	cp target/release/LICENSE target/release/LICENSING.md "$(OMNIVOX_INSTALL_BIN)/"
