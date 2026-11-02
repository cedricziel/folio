# document-processing Specification

## Purpose

The background pipeline that turns a stored file into a usable document: page count, thumbnails, OCR text, and a search embedding, driven by a persistent job queue.
## Requirements
### Requirement: Persistent job queue

The system SHALL keep background jobs in the `jobs` table with the states `pending`, `processing`, `completed` and `failed`. A single worker SHALL poll about once a second for the due pending job with the earliest scheduled time, mark it `processing`, and increment its attempt count.

#### Scenario: Jobs survive a restart

- **WHEN** the server stops while a job is pending
- **THEN** the job is picked up after the server starts again

### Requirement: Failed jobs are retried with backoff

When a job fails, the system SHALL schedule it again after `10 × 2^attempts` seconds while its attempts are below its maximum (3 by default), and SHALL mark it `failed` once the maximum is reached.

#### Scenario: First failure

- **WHEN** a job fails on its first attempt
- **THEN** it returns to `pending` and is scheduled 20 seconds later

#### Scenario: Last attempt fails

- **WHEN** a job fails on its third attempt
- **THEN** it is marked `failed` with the error recorded and is not retried

### Requirement: Stuck jobs are reset

Every 60 seconds the worker SHALL return to `pending` any job that has been `processing` for more than 5 minutes.

#### Scenario: Worker crashed mid-job

- **WHEN** a job has been `processing` for more than 5 minutes
- **THEN** it is set back to `pending` and processed again

### Requirement: Orphaned documents are repaired

On startup and every 5 minutes, the worker SHALL enqueue a processing job for each document that has a stored file, has no content, and has no pending or processing job.

#### Scenario: Document left without text

- **WHEN** a document with a file has no content and no active job
- **THEN** a processing job is enqueued for it

#### Scenario: Document already queued

- **WHEN** a document without content already has a pending job
- **THEN** no second job is enqueued

### Requirement: Processing a document

A processing job SHALL, in order: generate thumbnails and record the page count, extract text with OCR if the document has no content, and store an embedding if the document then has non-empty content. Failure of the thumbnail, OCR or embedding generation step SHALL be logged and SHALL NOT fail the job.

#### Scenario: PDF document

- **WHEN** a PDF is processed
- **THEN** its page count is stored and a thumbnail is generated for each of the first 50 pages

#### Scenario: Image document

- **WHEN** an image is processed
- **THEN** one thumbnail is generated and the page count is set to 1

#### Scenario: Local provider

- **WHEN** a document is processed while the provider is `local`
- **THEN** its thumbnails and page count are stored, its content stays empty, and no embedding is stored

#### Scenario: Document that already has content

- **WHEN** a document with existing content is processed
- **THEN** OCR is skipped and the existing content is used for the embedding

#### Scenario: Stored file is missing

- **WHEN** the document's file cannot be read
- **THEN** the job fails and is retried

### Requirement: OCR through the configured AI provider

OCR SHALL send the image to the provider selected in settings: `openai` (model `gpt-4o-mini`) or `anthropic` (model `claude-sonnet-4-20250514`). PDFs SHALL be rasterized with `pdftoppm` at 300 DPI and each page sent separately; page texts SHALL be joined with `--- Page Break ---` separators, and pages that fail are skipped.

#### Scenario: Multi-page PDF with OpenAI

- **WHEN** a three-page PDF is processed with the `openai` provider and a valid key
- **THEN** the document content is the three page texts joined by page-break separators

#### Scenario: Local provider

- **WHEN** the provider is `local`
- **THEN** OCR returns no text and the document keeps no content

#### Scenario: Provider selected without a key

- **WHEN** the provider is `openai` or `anthropic` and no API key is configured
- **THEN** OCR fails, the failure is logged, and the document keeps no content

#### Scenario: pdftoppm is not installed

- **WHEN** a PDF is processed and `pdftoppm` is not available
- **THEN** OCR fails with an error asking to install poppler-utils

### Requirement: Reprocessing on request

The system SHALL enqueue a processing job for a document at `POST /documents/:id/reprocess` and redirect to `/documents/:id`.

#### Scenario: Reprocess a document with a file

- **WHEN** a user reprocesses a document that has a stored file
- **THEN** a processing job is enqueued and the browser is redirected to the document page

#### Scenario: Reprocess a document without a file

- **WHEN** a user reprocesses a document that has no stored file
- **THEN** the response status is 400

### Requirement: Thumbnail serving

`GET /documents/:id/thumbnail` SHALL return the generated JPEG thumbnail when one exists, otherwise the original file for image documents, otherwise an SVG placeholder labelled `PDF` or `FILE`. Image thumbnails fit within 300 × 300 pixels; PDF thumbnails are page 1 rendered by `pdftoppm` with its longer side scaled to 800 pixels.

#### Scenario: Thumbnail not generated yet

- **WHEN** a PDF's thumbnail has not been generated
- **THEN** an SVG placeholder labelled `PDF` is returned

#### Scenario: Unknown document

- **WHEN** the document does not exist
- **THEN** the response status is 404

### Requirement: Page thumbnail serving

`GET /documents/:id/page/:page/thumbnail` SHALL return the generated JPEG for that page, or an SVG placeholder showing the page number when none exists.

#### Scenario: Page zero

- **WHEN** page 0 is requested
- **THEN** the response status is 400

#### Scenario: Page beyond the page count

- **WHEN** a page greater than the document's page count is requested
- **THEN** the response status is 404

### Requirement: Local OCRmyPDF provider

The system SHALL offer `ocrmypdf` through the OCR registry and existing settings selector. The provider SHALL recognize images and rendered PDF pages locally with the configured OCR languages, return per-page text with engine id `ocrmypdf`, and report CLI failures as OCR errors. The provider health check SHALL verify the CLI and installed Tesseract languages.

#### Scenario: Read a rendered PDF

- **WHEN** `ocrmypdf` is selected and a PDF page requires OCR
- **THEN** the page text is extracted through the local CLI and tagged with engine id `ocrmypdf`

#### Scenario: Missing language data

- **WHEN** the configured OCR language is not installed
- **THEN** the provider health check fails with the missing language

#### Scenario: Missing executable

- **WHEN** the OCRmyPDF CLI is not installed
- **THEN** the provider returns an actionable installation error

#### Scenario: No hosted provider

- **WHEN** `ocrmypdf` is selected without hosted AI credentials
- **THEN** recognition uses only local tools and requires no hosted AI request
