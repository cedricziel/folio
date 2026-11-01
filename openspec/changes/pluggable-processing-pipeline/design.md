## Context

`DynamicOcrService` and `DynamicEmbeddingService` matched on a single `ai.provider` string and hardcoded models. The worker treated NULL `content` as "not processed", which loops for providers that legitimately return no text.

## Goals / Non-Goals

**Goals:** pluggable providers per stage, a local default, tracked stage status, model-aware search.
**Non-Goals:** a native vector index (the `SearchService` port leaves room for one, e.g. pgvector), OCRmyPDF, per-page storage, layout analysis.

## Decisions

- **Registry of constructor functions** keyed by id (`Registry<dyn OcrService>`). Constructors receive resolved settings and return `Arc<dyn Trait>`. Chosen over dynamic loading for simplicity; adding a provider is one module plus one `register` line.
- **Text layer first** is a decorator (`TextLayerFirst`) around any OCR provider except `none`, so the fast path is provider independent.
- **`PageEngine` helper** lets engines implement "recognise one image"; PDF rendering at 300 DPI and page aggregation are shared.
- **Embeddings carry their model**: `embed` returns `EmbeddingBatch { vectors, model, dimensions }`. Search embeds the query and uses the returned model id, so the active model needs no separate lookup. Rows are tagged with `model` and a `stale` flag; `mark_stale_except` recomputes the flag, so switching back to a previous model restores its vectors.
- **Stage status in the database** (`pending|running|done|failed|skipped`) replaces "content IS NULL" as the repair criterion. Empty OCR output is stored as `""` with status `done`.
- **Separate jobs**: `ProcessDocument` (thumbnails) enqueues `ExtractText`, which enqueues `GenerateEmbedding`. Each stage retries independently.
- **Chunking** by characters (1500 with 200 overlap) at whitespace; token-accurate windows are unnecessary for retrieval at this scale.
- **Vectors as little-endian f32 blobs** in SQLite, scored in process; behind the port so a Postgres adapter can use pgvector.
- **Test connection** builds the provider from the unsaved form values and calls `health()`.
- **Migration**: legacy `ai.provider` maps to `ocr.provider` (`openai` to `openai-vision`, `anthropic` to `anthropic-vision`, otherwise `tesseract`) and `embeddings.provider` (`openai` or `none`). `ai.ocr_model` is copied only when it fits the provider; `ai.embedding_model` only for OpenAI.

## Risks / Trade-offs

- Upgrading re-embeds every document once because old vectors have no model id -> embedding stage is queued by the repair pass; with `embeddings.provider = none` documents are marked skipped.
- Full-scan cosine search does not scale to very large libraries -> acceptable now, replaceable behind the port.
- `fastembed` is untested in CI because it downloads models; it is compiled only with the cargo feature.
