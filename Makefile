.PHONY: build run test lint format clean dev setup apple apple-test apple-tokens apple-project

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

# Apple app: test FolioKit, generate the Xcode project, build for macOS
apple: apple-test apple-project
	cd apple/Folio && xcrun xcodebuild build -project Folio.xcodeproj -scheme Folio \
		-destination 'platform=macOS' -skipPackagePluginValidation -skipMacroValidation \
		CODE_SIGNING_ALLOWED=NO

apple-test:
	cd apple/FolioKit && xcrun swift test

# Generate apple/Folio/Folio.xcodeproj from project.yml (requires `brew install xcodegen`)
apple-project:
	cd apple/Folio && xcodegen generate

# Regenerate FolioTheme.swift and the color asset catalog from design/tokens.json
apple-tokens:
	cd apple/FolioKit && xcrun swift run folio-tokengen ../../design/tokens.json .
