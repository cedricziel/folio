## Context

The pipeline already renders PDFs into individual page images and joins per-page text. OCRmyPDF accepts a single image with explicit DPI and writes a text sidecar, as described in its [cookbook](https://ocrmypdf.readthedocs.io/en/latest/cookbook.html).

## Decisions

- Implement the existing page-engine adapter, using PNG and 300 DPI. Keep the PDF text-layer fast path in the pipeline wrapper.
- Run the CLI with standard PDF output, optimization disabled, and one worker. Generated PDFs, sidecar files, and CLI scratch files live in a temporary directory; its canonical path avoids macOS Tesseract failures through `/tmp` symlinks. The adapter returns the text and no confidence estimate.
- Reuse the Tesseract provider's configuration validation and language health check because OCRmyPDF uses Tesseract. Also check that the OCRmyPDF CLI is available.
- Test the actual CLI through the registry with a rendered generated PDF. CI installs the tool so that missing dependencies cannot silently skip coverage.

## Risks

- Distribution CLI versions differ. Use flags supported by the packaged Debian version and verify that version in a disposable container as well as the isolated local installation.
- CLI failures must propagate as OCR errors with stderr, so the worker can record failure and retry.
