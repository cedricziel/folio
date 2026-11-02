# Load Folio workflows through MCP

Connect to Folio using the [MCP connection guide](mcp.md). Folio serves the
`folio-archive` skill through HTTP and stdio. It teaches document search, OCR
reading, metadata changes, and uploads. The skill needs no AI provider on the
server.

## Use a host with skills support

Ask your host to load `skill://folio-archive/SKILL.md` from your Folio connection,
or select `folio-archive` from its discovered skills. Then ask, for example:

> Find the electricity invoices in Folio and summarize the amounts, citing each document.

For changes, state the action explicitly:

> Add the tax tag to the invoice we just reviewed, keeping its existing tags.

Host support varies. Connecting an MCP server does not guarantee skill support,
and reading a skill resource alone does not activate it. Consult your host's
skills-loading documentation and the [MCP skills extension](https://modelcontextprotocol.io/extensions/skills/overview).

## Implement a consumer

Use protocol revision `2026-07-28` or later and let your MCP SDK attach the required
per-request `_meta`, including protocol version and client information/capabilities.
The method parameters below omit this metadata for readability; they are not
complete wire requests.

1. Call `server/discover`. Check `capabilities.resources` and
   `capabilities.extensions["io.modelcontextprotocol/skills"]` before calling
   skills methods. Folio declares an empty extension object. It does not support
   `resources/directory/read`.
2. Call `skills/list` with `{}`. Folio returns the complete catalog in one page.
   Each entry includes `uri`, `frontmatter`, and `resources`. To load a known URI
   without listing, call `skills/get` with
   `{"uri":"skill://folio-archive/SKILL.md"}` and use `result.skill`.
3. Retain the complete entry with the originating server identity. Present any
   host-required approval against its full manifest. Do not fetch every file on
   connection, listing, or approval.
4. When loading the skill, call `resources/read` with
   `{"uri":"skill://folio-archive/SKILL.md"}` on that same server. Verify the UTF-8
   byte length and SHA-256 digest against its manifest before using the content.
   Parse the YAML frontmatter and compare every field with `frontmatter`.
5. Route verified instructions through your host's skill-loading path. Read
   `skill://folio-archive/references/workflows.md` only when needed, verifying it
   against the retained manifest too. Resolve relative references against the
   skill root, and reject files absent from the manifest.

List, get, and skill resource responses include `resultType: "complete"`,
`ttlMs: 300000`, and `cacheScope: "public"`. These files contain only bundled
instructions, never archive contents. HTTP transport authentication still applies;
reading instructions does not grant `documents:read` or `documents:write`.

On a digest, size, or frontmatter mismatch, reject the content and refresh the
entry with `skills/get`. A changed manifest invalidates persisted approval.
Keep cache keys and approvals bound to both server identity and skill URI.
Protect cached files against modification or reverify on every read, and keep
them outside filesystem skill discovery paths. Digests verify consistency with
the manifest, not the trustworthiness of the server. Treat skill content as
untrusted; require explicit approval for host-side execution or permission grants.
Cross-server reads and activation of nested skills require their own approval.

Unknown skill and skill-file URIs return JSON-RPC `-32602`. Folio rejects cursors
because the bundled catalog is returned in full. It never resolves skill URIs
against the host filesystem.

## Use a host without skills support

Use the [existing MCP tools](mcp.md#tools) directly. Start with `search_documents`,
read matches with `get_document`, and follow `next_cursor` for more text. Use
`add_tags` for incremental tagging and check `get_processing_status` after an
upload. These tools remain available without skills support.

You can read the bundled [workflow instructions](../crates/folio-mcp/skills/folio-archive/references/workflows.md)
as documentation. A client that can read resources can retrieve the known skill
URI as ordinary text, but must not treat that read as automatic activation or
permission to execute instructions.
