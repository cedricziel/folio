## MODIFIED Requirements

### Requirement: Semantic ranking

The system SHALL embed the query with the active embedding model, score every stored, non-stale chunk produced by that same model by cosine similarity, aggregate chunks per document by the best score, and return the highest-scoring documents first, up to a limit. Vectors produced by other models SHALL be ignored.

#### Scenario: Relevant document ranks first

- **WHEN** the embedding provider is `ollama` and the query is close in meaning to one document's text
- **THEN** that document is returned first with the highest score

#### Scenario: Local provider

- **WHEN** `embeddings.provider` is `none`
- **THEN** no query embedding is produced and no semantic results are returned

#### Scenario: Model changed

- **WHEN** the embedding model changes and a document has only vectors of the old model
- **THEN** it is not returned until it is re-embedded

#### Scenario: Embeddings disabled

- **WHEN** `embeddings.provider` is `none`
- **THEN** the query cannot be embedded and the web UI falls back to title search
