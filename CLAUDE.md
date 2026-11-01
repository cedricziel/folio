# CLAUDE.md

Folio is an AI-native document management system built on hexagonal architecture (ports in `folio-core`, adapters in `folio-infra`).

Settings live both in `config.toml` and in the database (managed via `/settings`).

## Conventions

- Use semantic commits
- Run `make format` and `make lint` before committing
- To check a change in the running app, use the `verify` skill (`.claude/skills/verify`): it launches an ephemeral instance, never your config
- `make e2e` runs the Playwright suite in `e2e/` against an ephemeral instance
