## ADDED Requirements

### Requirement: Trigger choices match runtime dispatch

The web and Apple workflow builders SHALL use the registered catalog. The dispatcher SHALL consider registered domain and legacy candidate names for an occurrence. Each dispatched candidate SHALL retain its selected trigger name in the run and payload. The workflow and mutation uniqueness constraint SHALL continue to prevent replay.

#### Scenario: An occurrence has two registered candidates

- **WHEN** a domain event and its legacy projection both map to selectable triggers
- **THEN** each corresponding workflow can run with the trigger it selected
- **AND** replay of the occurrence cannot queue additional runs

#### Scenario: Authorized catalog request

- **WHEN** a vault owner calls GET /api/workflows/triggers for the vault
- **THEN** Folio returns 200 with the same trigger choices shown on the web
