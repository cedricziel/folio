## ADDED Requirements

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
