## Why

OCR and embeddings are hard-wired to OpenAI and Anthropic with fixed model names, so the configured models are ignored (issue #8), the `local` provider produces no text and no embeddings, and documents with empty OCR are reprocessed every five minutes (issue #5). Users want documents scanned and embedded automatically, with a free local default and a way to plug in other engines.

## What Changes

- Introduce a provider registry per stage. OCR providers: `tesseract` (default), `openai-vision`, `anthropic-vision`, `ollama-vision`, `none`. Embedding providers: `openai` (any OpenAI-compatible endpoint), `ollama`, `fastembed` (cargo feature), `none` (default). Adding one is a module plus one registry line.
- Use an embedded PDF text layer (`pdftotext`) instead of OCR when it is substantial.
- Run OCR and embedding as separate tracked stages (`ocr_status`, `embed_status`) honouring `processing.auto_ocr` and `processing.auto_embed`.
- Split text into overlapping chunks; store vectors per chunk with model id and dimensions; search only compares vectors of the active model and aggregates per document. Offer "Re-embed all".
- **BREAKING**: replace `ai.provider`, `ai.ocr_model`, `ai.embedding_model` and `processing.auto_embedding` with `ocr.*`, `embeddings.*` and `processing.auto_embed`; a migration copies values. Existing embeddings are dropped and recomputed because they carry no model id.
- Add a Processing section to `/settings` with provider and model pickers, a test-connection button and Re-embed all.
- Ship `tesseract` (eng, deu) and `poppler-utils` in the container image.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `document-processing`: staged, status-tracked OCR and embedding; repair driven by stage status; providers chosen from a registry.
- `document-search`: chunk-level vectors restricted to the active embedding model.
- `settings`: new `ocr.*` / `embeddings.*` keys, honoured auto flags, processing section.

## Impact

- folio-core: `OcrOutput`, `EmbeddingBatch`, `SearchService` and `DocumentRepository` port changes; `Document` gains `ocr_status` and `embed_status`; settings models.
- folio-infra: `ai/` rewritten around `registry.rs`, `ocr/`, `embeddings/`; worker stages; SQLite repositories.
- folio-api: `/settings` routes and templates (HTML/Turbo only; no JSON API change except that upload now enqueues processing instead of running OCR inline); search handlers use the active model.
- Migration `005_pluggable_pipeline.sql` adds the status columns (backfilled from `content`), replaces `document_embeddings` with `document_chunks`, and seeds the new settings from the legacy `ai.*` keys. Settings live in the settings table; `config.toml` stays the fallback for API keys only.
- Docs: `docs/pipeline.md`. Dockerfile and CI install tesseract and poppler.
