## Why

Workflows currently select one event and edit one action of each kind. Users need ordered document actions, multiple events, date-based automation, and a native Apple builder.

## What changes

- Support multiple event triggers and durable scheduled, recurring, and document-date-offset triggers.
- Add bounded any/all/none condition groups for document properties, source, dates, and typed metadata.
- Add ordered rename, tag removal, classification clearing, metadata, review, and trash actions alongside existing actions.
- Provide equivalent web and Apple builders with validation, ordering, enable controls, and run history.
- Preserve immutable run inputs, retries, webhook credential redaction, vault boundaries, and workflow-origin loop suppression.
- Integrate review/version events and metadata/date contracts after their owning slices merge. Email notifications remain excluded unless existing delivery configuration supports them.

## Impact

Touches folio-core models and ports, folio-infra database adapters and workers, folio-api JSON and HTML routes/templates, OpenAPI, and the generated Swift client and native views. Existing JSON workflow routes and HTML workflow routes gain the expanded definition.

New SQLite and PostgreSQL migrations will add trigger and schedule persistence without editing old migrations. Existing single-trigger definitions retain their meaning. Exact migration names and dependency revisions are recorded before implementation. No new global configuration setting is planned.

Foundation lifecycle and vault ownership must merge first. Typed metadata and document date/review/version work must merge before their workflow integration.

## Domain event amendment

Consume the foundation's versioned domain event envelope through the existing
durable outbox. Expose meaningful committed transitions from each domain slice
as selectable workflow triggers, with labels and condition data in both builders.
Distinguish document subjects from non-document subjects and reject actions that
cannot apply. Enforce event visibility before workflow matching, including
private saved views and restricted discussions. Preserve event identity,
occurrence time, vault, subject, nullable actor, and correlation/causation.
