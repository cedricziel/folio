## Why

Issue #46 requests a selectable local OCRmyPDF provider alongside the existing OCR engines.

## What Changes

- Register `ocrmypdf` as a local, language-aware OCR engine.
- Reuse page rendering and extract OCRmyPDF's sidecar text from temporary files.
- Install the CLI in the container and OCR CI job, and document selection and verification.

## Capabilities

### Modified Capabilities

- `document-processing`: Add local OCRmyPDF extraction for images and rendered PDF pages.

## Impact

Touches folio-infra's OCR adapter and registry, packaging, and provider documentation. Uses the existing `OcrService` port and `ocr.provider` setting in the database and startup configuration. No new settings, database schema, HTTP endpoints, OpenAPI changes, or Swift client changes. The existing HTML settings selector lists registry providers automatically.
