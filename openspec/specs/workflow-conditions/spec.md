# workflow-conditions Specification

## Purpose
TBD - created by archiving change workflow-condition-groups. Update Purpose after archive.
## Requirements
### Requirement: Bounded grouped workflow conditions
The system SHALL evaluate legacy condition fields conjunctively with nested all, any, and none groups against document, historical event, and ingestion-candidate metadata.

#### Scenario: Alternative conditions
- **WHEN** a condition contains an any group
- **THEN** at least one branch must match, and an explicitly empty group never matches

#### Scenario: Exclusions and conjunctions
- **WHEN** a condition contains all or none groups
- **THEN** every all branch must match and no none branch may match

#### Scenario: Excessive nesting or size
- **WHEN** POST /api/workflows or PUT /api/workflows/{id} contains more than eight nested group levels or 64 condition objects
- **THEN** the API returns 400 without saving the definition

#### Scenario: Nested invalid source pattern
- **WHEN** POST /api/workflows includes an invalid regular expression inside a group
- **THEN** the API returns 400 without saving the definition

### Requirement: Lossless condition editing
The system SHALL preserve grouped conditions when the existing HTML editor updates a workflow's other fields.

#### Scenario: HTML edit of an API-created group
- **WHEN** a user saves the existing HTML form for a workflow created with grouped conditions
- **THEN** GET /api/workflows/{id} returns 200 with the retained groups and the updated form fields

