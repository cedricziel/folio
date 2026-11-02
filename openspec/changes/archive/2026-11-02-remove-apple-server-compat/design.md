# Design

Require successful vault discovery before exposing vault contents or write capabilities. An HTTP error from `GET /api/vaults`, including 404, remains an error. An unloaded catalog represents pending state and cannot enable an alternate server mode.

File Provider roots list vault folders. Resolve contents only with an accessible vault and reject old unscoped identifiers. Keep transient discovery failures distinct from successful empty enumeration so a network error cannot imply deletion.

External Spotlight document identifiers contain both vault and document UUIDs. Reject UUID-only identifiers and arbitrary strings ending in a UUID. Keep intentional internal default-vault operations, including current App Intent lookup, separate from external identifier parsing.

The JSON API and generated OpenAPI schema do not change. Existing SQL migrations remain untouched.
