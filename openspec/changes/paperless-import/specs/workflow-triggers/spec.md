## ADDED Requirements

### Requirement: Import and sync changes don't start workflows

Workflows SHALL NOT start from document events produced by an import or a Paperless sync (created, updated, title, tag, correspondent or type changes, trashed, restored). Workflows SHALL NOT start from `document.processed` for documents whose source is `import`. Changes that users or workflows make to imported documents SHALL start workflows as usual.

#### Scenario: Upload workflow and an import
- **WHEN** a vault has an enabled workflow on `document.uploaded` and an import creates documents in it
- **THEN** no workflow run is created for the imported documents

#### Scenario: Processing workflow and an import
- **WHEN** a vault has an enabled workflow on `document.processed` and an imported document finishes thumbnail and embedding processing
- **THEN** no workflow run is created for that document

#### Scenario: Mirror sync changes tags
- **WHEN** a mirror sync changes the tags of an imported document and a workflow listens on `document.tags_changed`
- **THEN** no workflow run is created

#### Scenario: Later user edits still trigger
- **WHEN** a user adds a tag to an imported document and a workflow listens on `document.tags_changed`
- **THEN** the workflow runs
