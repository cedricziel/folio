## ADDED Requirements

### Requirement: Atomic workflow document actions

Folio SHALL commit the document mutation, its outbox event, and the run’s completed-action count in one transaction. It SHALL check the workflow revision and vault before mutating the document. Job retries SHALL continue from committed progress. The Move to Trash action SHALL be last.

#### Scenario: Progress cannot be written

- **WHEN** an action fails to write progress after changing a document within its transaction
- **THEN** Folio rolls back the document change, outbox event, and progress

#### Scenario: A runner restarts after a completed rename

- **WHEN** someone edits the title after the rename commits and a runner resumes the workflow
- **THEN** Folio retains the later title and starts at the next action

#### Scenario: A grouped action references another vault

- **WHEN** a workflow tries to add tags and one tag belongs to another vault
- **THEN** Folio applies none of the tag changes
