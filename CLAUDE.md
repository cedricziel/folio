# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Build & Development Commands

```bash
make build          # Build the project
make run            # Run the application
make dev            # Run with auto-reload (requires cargo-watch)
make test           # Run all tests
make lint           # Run clippy with warnings as errors
make format         # Format code
make check          # Run lint + format-check + test
make migrate        # Run database migrations
make setup          # Install dev tools (cargo-watch, sqlx-cli)
```

Run a single test: `cargo test --package folio-core test_name`

## Architecture

Folio is an AI-native document management system using **Hexagonal Architecture** (Ports & Adapters):

```
folio-app/          # Binary - wires everything together, starts server
crates/
  folio-core/       # Domain layer - models, ports (traits), business logic
  folio-infra/      # Infrastructure - SQLite repos, AI providers, file storage
  folio-api/        # HTTP layer - Axum routes, templates, SSE events
```

### Key Patterns

- **Ports (traits)** defined in `folio-core/src/ports/`: `DocumentRepository`, `TagRepository`, `OcrService`, `EmbeddingService`, `SearchService`, `SettingsRepository`
- **Adapters** implemented in `folio-infra/`: SQLite repositories, OpenAI/Anthropic/Local AI providers
- **Background jobs**: SQLite-backed queue in `folio-infra/src/queue/` with Worker processing OCR, thumbnails, embeddings
- **Real-time updates**: SSE via `EventBroadcaster` - worker emits events, clients receive via `/events` endpoint
- **Web UI**: Server-rendered HTML with Turbo + Stimulus (in `base.html`), templates use Askama

### Configuration

- `config.toml` for server/database/storage/AI settings
- Environment variables: `FOLIO_AI_OPENAI_API_KEY`, `FOLIO_AI_ANTHROPIC_API_KEY`
- Settings also stored in database (managed via `/settings` page)

### Database

SQLite with migrations in `migrations/`. Tables: `documents`, `tags`, `document_tags`, `correspondents`, `document_types`, `document_embeddings`, `jobs`, `settings`.

## Conventions

- Use semantic commits
- Run `make format` and `make lint` before committing
