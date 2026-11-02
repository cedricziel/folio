# MCP skills specification

## Purpose
Let MCP consumers discover and load verified Folio archive workflow instructions from the server.

## Requirements
### Requirement: Discover archive skills
Folio SHALL declare io.modelcontextprotocol/skills and resources capabilities and return complete skill entries with unchanged frontmatter, complete manifests, resultType, ttlMs, and cacheScope.

#### Scenario: Discover without an AI provider
- **WHEN** a consumer calls skills/list with no AI provider configured
- **THEN** Folio returns the bundled folio-archive skill with SHA-256 digests and byte sizes for every file

### Requirement: Retrieve skill instructions
Folio SHALL support direct skills/get lookup and resources/read for every manifest URI without requiring prior listing or document access.

#### Scenario: Load by URI
- **WHEN** a consumer requests skill://folio-archive/SKILL.md through skills/get and resources/read
- **THEN** the metadata matches the frontmatter and the returned bytes match the manifest

#### Scenario: Reject unknown content
- **WHEN** a consumer requests an unknown skill or a file outside the bundled manifest
- **THEN** Folio returns JSON-RPC -32602 without reading local files

### Requirement: Teach archive workflows
The bundled skill SHALL describe search, paginated OCR reading, metadata changes, and asynchronous uploads using existing tools.

#### Scenario: Consumer without skills support
- **WHEN** a consumer cannot load MCP skills
- **THEN** the documentation explains using existing tools directly without claiming automatic skill activation

