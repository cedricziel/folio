## ADDED Requirements

### Requirement: Imported documents don't start upload or processing workflows

Workflows SHALL NOT start from `document.uploaded` or `document.processed` events for documents whose source is `import`. Other document events for those documents, such as tag, title, correspondent or type changes made after the import, SHALL start workflows as usual.

#### Scenario: Upload workflow and an import
- **WHEN** a vault has an enabled workflow on `document.uploaded` and an import creates documents in it
- **THEN** no workflow run is created for the imported documents

#### Scenario: Processing workflow and an import
- **WHEN** a vault has an enabled workflow on `document.processed` and an imported document finishes thumbnail and embedding processing
- **THEN** no workflow run is created for that document

#### Scenario: Later edits still trigger
- **WHEN** a user adds a tag to an imported document and a workflow listens on `document.tags_changed`
- **THEN** the workflow runs
