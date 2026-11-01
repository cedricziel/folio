# MCP server

Folio ships a [Model Context Protocol](https://modelcontextprotocol.io) server so AI assistants can search and organize your archive.

- **HTTP:** `http://localhost:3000/mcp` (streamable HTTP), served by the main Folio process.
- **stdio:** `folio-app mcp --stdio`, for clients that launch a local process.

## Connect

Claude Code:

```sh
claude mcp add --transport http folio http://localhost:3000/mcp
```

Claude Desktop (stdio), in `claude_desktop_config.json`:

```json
{
  "mcpServers": {
    "folio": {
      "command": "/path/to/folio-app",
      "args": ["mcp", "--stdio"],
      "cwd": "/path/to/folio"
    }
  }
}
```

Run it from the directory that holds `config.toml`, or set the `FOLIO_*` environment variables. Logs go to stderr. Uploads made over stdio are processed by the background worker of that process, so OCR only runs while the client is connected.

## Tools

| Tool | Purpose | Access |
| --- | --- | --- |
| `search_documents` | Find documents by words, tags, correspondent, type, and date range | read |
| `get_document` | Metadata and OCR text, paginated with a cursor | read |
| `list_tags`, `list_correspondents`, `list_document_types` | The vocabulary used for filters and updates | read |
| `get_processing_status` | Background processing state of a document | read |
| `update_document` | Change title, correspondent, type, or replace tags | write |
| `add_tags`, `remove_tags` | Adjust tags without replacing them | write |
| `upload_document` | Add a file (base64, max 25 MiB) | write |

There is no delete tool. Tools carry MCP annotations (`readOnlyHint`, `destructiveHint`, `idempotentHint`) so clients can ask for confirmation sensibly.

Resources: `folio://documents/{id}` (metadata and text) and `folio://documents/{id}/file` (original file as a blob, up to 5 MiB).

The `date` returned and filtered on is the date a document was added to Folio.

## Configuration

```toml
[mcp]
enabled = true            # serve /mcp
allowed_hosts = []        # extra Host headers to accept, e.g. ["folio.example.com"]
```

The `mcp.enabled` setting stored in the database (`/settings`) takes precedence over `config.toml` and applies without a restart. `FOLIO_MCP__ENABLED` overrides the file.

By default only loopback `Host` headers are accepted, which protects a local server from DNS rebinding. List other hostnames in `allowed_hosts` when you reach Folio through a reverse proxy.

## Authentication

The endpoint does not authenticate callers yet. Do not expose `/mcp` beyond localhost until OAuth is wired in: `folio_mcp::router(..)` returns an axum router, so an auth layer is added with `.layer(..)` in `folio-app`. The layer validates the bearer token and inserts a `folio_mcp::McpAuth` (subject and scopes) into the request extensions. When present, read tools need the `documents:read` scope and write tools `documents:write`. Without an `McpAuth` extension, as with stdio, every tool is allowed.
