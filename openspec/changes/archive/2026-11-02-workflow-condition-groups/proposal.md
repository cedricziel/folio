## Why

Workflow conditions only support a flat conjunction. Alternative matches and exclusions need any/all/none groups, with limits that prevent unbounded evaluation. The current HTML form must retain groups saved through the JSON API.

## What changes

- Add bounded nested groups while preserving existing leaf fields and wire formats.
- Validate nested source patterns and preserve grouped conditions during HTML edits.

## Impact

Touches folio-core models, folio-api HTML form handling, JSON workflow schemas, and generated Swift client input through OpenAPI. Existing JSON POST/PUT /api/workflows and HTML POST /workflows/{id} retain their paths. Existing JSON persistence stores the additional condition fields without a migration. No settings change. Domain-event consumption and builder controls remain separate increments.
