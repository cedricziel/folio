.PHONY: build run test lint format clean dev setup openapi

# Default target
all: build

# Build the project
build:
	cargo build

# Build for release
release:
	cargo build --release

# Run the application
run:
	cargo run --bin folio-app

# Run in development mode with auto-reload
dev:
	cargo watch -x 'run --bin folio-app'

# Run tests
test:
	cargo test --workspace

# Run linter
lint:
	cargo clippy --workspace --all-targets -- -D warnings

# Format code
format:
	cargo fmt --all

# Regenerate openapi/openapi.json from the Rust code
openapi:
	UPDATE_OPENAPI=1 cargo test -p folio-api --test openapi

# Check formatting
format-check:
	cargo fmt --all -- --check

# Clean build artifacts
clean:
	cargo clean

# Setup development environment
setup:
	@echo "Installing required tools..."
	cargo install cargo-watch
	cargo install sqlx-cli --no-default-features --features sqlite
	@echo "Creating data directory..."
	mkdir -p data
	@echo "Setup complete!"

# Run database migrations
migrate:
	sqlx migrate run --source migrations

# Create a new migration
migrate-new:
	@read -p "Migration name: " name; \
	sqlx migrate add -r $$name --source migrations

# Check project (lint + test + format-check)
check: lint format-check test
	@echo "All checks passed!"
