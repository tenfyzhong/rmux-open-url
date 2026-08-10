CARGO ?= cargo
CARGO_HOME ?= $(HOME)/.cargo
INSTALL_DIR ?= $(CARGO_HOME)/bin
BINARY := rmux-open-url
RELEASE_BINARY := target/release/$(BINARY)

.DEFAULT_GOAL := build

.PHONY: build release fmt fmt-check lint test check install clean help

build:
	$(CARGO) build --locked

release:
	$(CARGO) build --release --locked

fmt:
	$(CARGO) fmt

fmt-check:
	$(CARGO) fmt -- --check

lint:
	$(CARGO) clippy --all-targets --all-features -- -D warnings

test:
	$(CARGO) test --locked

check: fmt-check lint test

install: release
	install -d "$(INSTALL_DIR)"
	install -m 0755 "$(RELEASE_BINARY)" "$(INSTALL_DIR)/$(BINARY)"

clean:
	$(CARGO) clean

help:
	@echo "Available targets:"
	@echo "  build      Build the debug binary (default)"
	@echo "  release    Build the optimized release binary"
	@echo "  fmt        Format Rust sources"
	@echo "  fmt-check  Check Rust formatting"
	@echo "  lint       Run Clippy with warnings denied"
	@echo "  test       Run all tests"
	@echo "  check      Run formatting, lint, and tests"
	@echo "  install    Build release and install to $(INSTALL_DIR)"
	@echo "  clean      Remove Cargo build artifacts"
