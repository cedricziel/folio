# MCP server

Folio ships a [Model Context Protocol](https://modelcontextprotocol.io) server so AI assistants can search and organize your archive.

- **HTTP:** `http://localhost:3000/mcp` (streamable HTTP), served by the main Folio process. On by default while `auth.required` is true; see [Authentication](#authentication).
- **stdio:** `folio-app mcp --stdio`, for clients that launch a local process.

## Connect

Claude Code, Claude Desktop and claude.ai connect by URL alone:

```sh
claude mcp add --transport http folio https://folio.example.com/mcp
```

The client finds Folio's authorization server, registers itself, and opens a browser for you to sign in and allow access. See [auth.md](auth.md) for the first-run setup and `server.public_url`.

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
# enabled = true         # serve /mcp; unset follows auth.required
allowed_hosts = []        # extra Host headers to accept, e.g. ["folio.example.com"]
```

The `/settings` page exposes the `mcp.enabled` toggle. Its value is cached and refreshed when settings change. The setting stored in the database takes precedence over `config.toml` and applies without a restart. `FOLIO_MCP__ENABLED` overrides the file.

By default only loopback `Host` headers are accepted, which protects a local server from DNS rebinding. List other hostnames in `allowed_hosts` when you reach Folio through a reverse proxy.

## Authentication

`/mcp` requires an OAuth 2.1 bearer token from Folio's own authorization server (see [auth.md](auth.md)). A request without a valid token gets `401` with `WWW-Authenticate: Bearer resource_metadata="<public_url>/.well-known/oauth-protected-resource"`, which is how MCP clients discover where to log in. Tokens must be issued for the `/mcp` resource.

Read tools need the `documents:read` scope and write tools `documents:write`; clients get both unless the user narrows them. The validated caller reaches the tools as a `folio_mcp::McpAuth` request extension. stdio has no authentication: the client is a local process you started.

With `auth.required = false` the endpoint is open and Folio logs a warning at startup; do not expose that beyond localhost.

Processing status reports active work first. Once no jobs are active, the latest finished job for each processing stage determines whether processing completed or failed. A successful retry clears an earlier failure in that stage; unrelated successful work does not. Failed file storage removes the newly created document record.
