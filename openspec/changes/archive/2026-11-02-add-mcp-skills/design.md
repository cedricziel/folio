## Design

Embed one folio-archive skill and its workflow reference into the binary. Use rmcp custom request handling for skills/list and skills/get, and existing resource handling for content. Publish the extension through ServerConfig so server/discover advertises it. Omit optional directory reading.

Keep the embedded YAML frontmatter in JSON flow-mapping syntax so serde_json can preserve every field without adding a YAML runtime dependency. Parse it from SKILL.md and compute manifests from the exact embedded bytes. Return a complete catalog without pagination; reject supplied cursors. Match URIs exactly, without filesystem access. Bundled instructions are public, contain no archive data, and require no document scope. Existing transport authentication and document tool/resource authorization remain unchanged.

Use in-memory MCP transport tests to exercise discovery, direct lookup, manifest integrity, resource content, and invalid requests. Existing HTTP and OAuth tests cover transport boundaries. No AI provider is needed.
