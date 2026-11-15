## ADDED Requirements

### Requirement: Integration navigation

The web application SHALL expose API docs and MCP server shortcuts in shared desktop and mobile navigation.

#### Scenario: Open API docs from a selected vault
- **WHEN** a user follows API docs while browsing a vault
- **THEN** the browser opens `/api/docs` without a vault prefix

#### Scenario: Inspect MCP connection details
- **WHEN** a user follows MCP server
- **THEN** `/settings/integrations` shows the configured absolute MCP URL, a copy action, and OAuth and personal-token connection guidance

#### Scenario: MCP is disabled
- **WHEN** the effective MCP setting disables the server
- **THEN** the page shows that state without enabling the server or presenting it as available

### Requirement: Safe token controls

The integrations page SHALL let signed-in users manage their own tokens and SHALL show a newly created secret only once.

#### Scenario: Copy a new token
- **WHEN** token creation succeeds
- **THEN** the page displays a copyable secret without storing it in browser storage or URL parameters

#### Scenario: Revisit the page
- **WHEN** a user reloads or revisits integrations
- **THEN** token metadata remains visible and the secret is not recoverable
