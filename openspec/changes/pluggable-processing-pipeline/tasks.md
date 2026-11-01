## 1. Ports and models

- [x] 1.1 Add `OcrOutput`, `EmbeddingBatch`, `EmbeddedChunk`, stage status and pipeline settings; test: `reads_pipeline_keys`
- [x] 1.2 Migration `005_pluggable_pipeline.sql` and repository methods; test: search and worker tests run on the migrated schema

## 2. Providers

- [x] 2.1 Registry and `none`, `tesseract`, `openai-vision`, `anthropic-vision`, `ollama-vision`; tests: `recognises_text_in_a_generated_png`, `uses_the_configured_model`
- [x] 2.2 Embedding providers `openai`, `ollama`, `fastembed`; tests: `embeds_with_the_configured_model_and_reports_dimensions`, `embeds_a_batch`
- [x] 2.3 PDF text-layer fast path; test: `text_layer_skips_the_engine`
- [x] 2.4 Chunker; tests in `chunker.rs`
- [x] 2.5 Chunk store with stale handling; test: `switching_models_marks_old_vectors_stale_and_switching_back_restores_them`

## 3. Pipeline

- [x] 3.1 Staged worker honouring `processing.auto_ocr` and `processing.auto_embed`; tests: `auto_flags_are_honoured`, `ocr_then_embedding_run_in_sequence_and_store_model_tagged_chunks`
- [x] 3.2 Issue #5 regression; test: `empty_ocr_result_is_stored_and_not_requeued`
- [x] 3.3 Upload enqueues processing instead of running OCR inline

## 4. Settings UI

- [x] 4.1 Processing section, test connection, Re-embed all

## 5. Packaging and docs

- [x] 5.1 Dockerfile and CI install tesseract and poppler
- [x] 5.2 `docs/pipeline.md`
- [ ] 5.3 Run `make format`, `make lint`, `make test`
