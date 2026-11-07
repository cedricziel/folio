# Folio

Folio is a self-hosted, AI-native document management system. Upload or feed
it documents, and it extracts their text and makes them searchable by meaning.
It runs as one binary with SQLite and local storage by default.

- OCR with local Tesseract or a vision model ([pipeline](docs/pipeline.md))
- Semantic search over embeddings
- Automatic ingestion from a watched folder or IMAP mailboxes ([ingestion](docs/ingestion.md))
- Workflows that call webhooks on document events ([workflows](docs/workflows.md))
- An MCP server for AI clients ([mcp](docs/mcp.md))
- Sign-in with passwords, OpenID Connect or forward auth ([auth](docs/auth.md))
- SQLite or PostgreSQL, local disk or S3

## Quick start

```sh
docker run -d --name folio -p 3000:3000 -v folio-data:/data \
  -e FOLIO_SERVER__PUBLIC_URL=http://localhost:3000 \
  ghcr.io/cedricziel/folio:latest
docker logs folio   # open the /setup link it prints
```

For a real deployment, with a reverse proxy, upgrades and backups, read
[Deploy your own Folio](docs/deployment.md).

## Documentation

- [Configuration](docs/configuration.md)
- [Authentication](docs/auth.md)
- [Vault selection](docs/vaults.md)
- [Importing from Paperless-ngx](docs/paperless-import.md)
- [Database](docs/database.md) and [storage](docs/storage.md)
- [Observability](docs/observability.md)
- [Keyboard shortcuts](docs/shortcuts.md)

## Apple apps

A native app for iOS, iPadOS and macOS connects to your server. See
[apple/README.md](apple/README.md).

## License

[AGPL-3.0](LICENSE)
