# Require the current server contract in Apple clients

Apple clients currently interpret a missing vault discovery endpoint as an older server and expose an unscoped File Provider tree. External document identifiers also accept UUID-only values. Remove these compatibility paths so failures remain visible and external document identity includes its vault.

This change touches Apple networking, navigation, File Provider models, and their tests. It does not change folio-core, folio-infra, or folio-api endpoints. The current server behavior for requests without `X-Folio-Vault` remains supported. Global AI providers and database migration history remain unchanged. No new settings or migrations are needed.
