.PHONY: build run test lint format clean dev setup tokens assets openapi apple apple-test apple-tokens apple-project e2e e2e-update-snapshots

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

# Regenerate static/tokens.css and static/components.css from design/
tokens:
	cargo run -p xtask -- tokens

# Re-copy vendored scripts and fonts from npm into static/
assets:
	npm ci --prefix web
	npm run vendor --prefix web

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

# End-to-end tests (Playwright) against an ephemeral instance; rebuilds folio-app first
e2e: build
	cd e2e && npm ci && npx playwright install chromium && npx playwright test

# Regenerate the visual baselines in e2e/tests/__screenshots__ on Linux, as CI renders them.
# Builds folio-app in a Rust container (target-linux/), then runs the visual spec in the
# official Playwright image pinned to e2e's @playwright/test version. amd64 matches CI;
# on Apple silicon this runs under emulation and takes a while.
PLAYWRIGHT_VERSION = $(shell node -p "require('./e2e/package.json').devDependencies['@playwright/test']")
DOCKER_PLATFORM ?= linux/amd64
e2e-update-snapshots:
	docker run --rm --platform $(DOCKER_PLATFORM) -v "$(CURDIR)":/src -w /src \
		-e CARGO_TARGET_DIR=/src/target-linux rust:1-bookworm cargo build --locked -p folio-app
	docker run --rm --platform $(DOCKER_PLATFORM) --ipc=host -v "$(CURDIR)":/src -v /src/e2e/node_modules \
		-w /src/e2e -e FOLIO_BIN=/src/target-linux/debug/folio-app -e FOLIO_REPO=/src \
		mcr.microsoft.com/playwright:v$(PLAYWRIGHT_VERSION)-noble \
		bash -c "apt-get update -qq && apt-get install -y -qq poppler-utils >/dev/null && npm ci && npx playwright test visual.spec.ts --update-snapshots"
