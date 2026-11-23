## ADDED Requirements

### Requirement: Capture processing outcomes in workflows

Workflow processing triggers SHALL use the registered stage and outcome names. The engine SHALL evaluate available processing fields and document metadata from the committed occurrence. It SHALL preserve absent source data, require an active parent in the event vault, and suppress workflow and import origins. Replaying the occurrence SHALL NOT create another run.

#### Scenario: OCR completion without captured source

- **WHEN** a committed OCR completion captures a document title and has_text flag but no source
- **THEN** a matching workflow receives the captured title and processing fields
- **AND** a workflow requiring a browser-upload source does not match
