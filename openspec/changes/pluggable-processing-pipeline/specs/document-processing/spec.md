## MODIFIED Requirements

### Requirement: Orphaned documents are repaired

On startup and every 5 minutes, the worker SHALL enqueue a processing job for each document that has a stored file, whose `ocr_status` is `pending` and that has no pending or processing job. It SHALL enqueue an embedding job for each document whose `embed_status` is `pending`, that has non-empty content and no active job, and SHALL mark `embed_status` `skipped` when OCR has finished without text.

#### Scenario: Document never processed

- **WHEN** a document with a file has `ocr_status` `pending` and no active job
- **THEN** a processing job is enqueued for it

#### Scenario: Document left without text

- **WHEN** a document with a file has `ocr_status` `pending` and no content
- **THEN** a processing job is enqueued for it

#### Scenario: Document already queued

- **WHEN** a pending document already has a pending job
- **THEN** no second job is enqueued

#### Scenario: OCR found no text

- **WHEN** OCR succeeded and returned no text
- **THEN** the content is stored as an empty string, `ocr_status` is `done`, and the repair pass never enqueues the document again

### Requirement: Processing a document

A processing job SHALL generate thumbnails and record the page count, then, if `processing.auto_ocr` is true or the document already has content, mark `ocr_status` `pending` and enqueue an extract-text job; otherwise it SHALL mark `ocr_status` `skipped`. The extract-text job SHALL set `ocr_status` to `running`, then `done` on success (storing the text, possibly empty) or `failed` on error. After a successful OCR with text, if `processing.auto_embed` is true and the embedding provider is not `none`, an embedding job SHALL be enqueued; otherwise `embed_status` SHALL be `skipped`. Thumbnail failures SHALL be logged and SHALL NOT fail the job; OCR and embedding failures SHALL fail their own job so it is retried.

#### Scenario: PDF document

- **WHEN** a PDF is processed
- **THEN** its page count is stored and a thumbnail is generated for each of the first 50 pages

#### Scenario: Image document

- **WHEN** an image is processed
- **THEN** one thumbnail is generated and the page count is set to 1

#### Scenario: OCR disabled

- **WHEN** `processing.auto_ocr` is false and a document without content is processed
- **THEN** `ocr_status` is `skipped` and no OCR job is enqueued

#### Scenario: Embedding disabled or unconfigured

- **WHEN** `processing.auto_embed` is false or `embeddings.provider` is `none`
- **THEN** `embed_status` is `skipped` and no embedding job is enqueued

#### Scenario: Local provider

- **WHEN** a document is processed with `ocr.provider` `tesseract` or `none` and `embeddings.provider` `none`
- **THEN** its thumbnails and page count are stored, no embedding is stored, and `embed_status` is `skipped`

#### Scenario: Document that already has content

- **WHEN** a document with existing content is processed
- **THEN** OCR is skipped, `ocr_status` is `done`, and the existing content is used for the embedding

#### Scenario: Stored file is missing

- **WHEN** the document's file cannot be read during OCR
- **THEN** `ocr_status` becomes `failed` and the job is retried

### Requirement: OCR through the configured AI provider

OCR SHALL use the provider selected by `ocr.provider` from the registry: `tesseract` (default), `ocrmypdf`, `openai-vision`, `anthropic-vision`, `ollama-vision` or `none`, with `ocr.model` (empty selects the provider default), `ocr.languages` and `ocr.base_url`. For PDFs, an embedded text layer extracted with `pdftotext` SHALL be used instead of the provider when at least 80% of pages contain 20 or more letters or digits. Otherwise pages SHALL be rasterized with `pdftoppm` at 300 DPI and recognised page by page, joined by blank lines; a failing page fails the OCR. Each result SHALL report the engine id and, when available, a confidence.

#### Scenario: PDF with a text layer

- **WHEN** a PDF with a substantial text layer is processed
- **THEN** the text layer is stored and the OCR provider is not called

#### Scenario: Scanned PDF with tesseract

- **WHEN** a scanned two-page PDF is processed with `ocr.provider` `tesseract` and `ocr.languages` `deu+eng`
- **THEN** both pages are rasterized and recognised and the content holds both page texts

#### Scenario: Configured model is used

- **WHEN** `ocr.provider` is `openai-vision` and `ocr.model` is `my-model`
- **THEN** requests name `my-model`

#### Scenario: Multi-page PDF with OpenAI

- **WHEN** a three-page PDF without a text layer is processed with `openai-vision` and a valid key
- **THEN** the document content is the three page texts joined by blank lines

#### Scenario: Local provider

- **WHEN** `ocr.provider` is `tesseract`
- **THEN** no hosted AI provider is called

#### Scenario: OCR disabled provider

- **WHEN** `ocr.provider` is `none`
- **THEN** no text is extracted and `ocr_status` is `skipped`

#### Scenario: Provider selected without a key

- **WHEN** a hosted provider is selected without an API key and without a custom base URL
- **THEN** OCR fails with an error and `ocr_status` is `failed`

#### Scenario: pdftoppm is not installed

- **WHEN** a PDF is processed and `pdftoppm` is not available
- **THEN** OCR fails with an error asking to install poppler-utils

## ADDED Requirements

### Requirement: Embedding stage

The embedding job SHALL split the content into overlapping windows of at most 1500 characters with 200 characters of overlap, embed them in batches with the provider selected by `embeddings.provider` (`openai`, `ollama`, `fastembed`, `none`), and replace the document's stored chunks, each tagged with the model id and dimensions. Before storing, vectors of any other model SHALL be marked stale.

#### Scenario: Long document

- **WHEN** a 4000-character document is embedded
- **THEN** several overlapping chunks are stored with the model id

#### Scenario: OpenAI-compatible endpoint

- **WHEN** `embeddings.provider` is `openai` and `embeddings.base_url` points at a local server
- **THEN** requests go to `<base_url>/embeddings` and no API key is required

### Requirement: Provider health check

Every provider SHALL offer a health check that reports whether it is usable with the current settings.

#### Scenario: Ollama model not pulled

- **WHEN** the health check runs against an Ollama server that lacks the configured model
- **THEN** it fails with a message naming the model and `ollama pull`
