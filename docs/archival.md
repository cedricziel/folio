# PDF/A archival generation

Folio can generate a searchable PDF/A-2b copy of a verified PDF, JPEG, PNG or
TIFF original. Processing runs in a separate worker lane, with one conversion
per Folio process. Originals remain downloadable while conversion runs and
after a failed or skipped attempt. OCR and embedding settings are independent
of this lane.

The official container image enables automatic discovery for new uploads and
existing verified originals without an attempt. Native installations default
to off. Set `FOLIO_ARCHIVAL__AUTOMATIC=false` in the container to override the
default, or set it to `true` to opt in on a native installation. Already queued
attempts resume when automatic discovery is disabled.

The worker picks up eligible originals in bounded batches when the archive
queue is idle. Legacy originals become eligible after original verification
has recorded their identity. Upgrading a large archive can therefore start
a sustained backfill. Conversion and validation use CPU, memory and temporary
disk, and completed PDF/A copies add to storage usage. During an upgrade from
a release without archival jobs, stop every older worker sharing the database
before starting any upgraded worker or admitting archive jobs. Older workers
cannot decode the new job type.

The container includes OCRmyPDF, Ghostscript, Poppler, qpdf and veraPDF. In a
native installation, make those tools and Java available on the Folio
process's PATH. Set `FOLIO_ARCHIVAL__VERAPDF` to an absolute executable path
if veraPDF lives elsewhere. Set `FOLIO_ARCHIVAL__LANGUAGES` to the installed
Tesseract languages, such as `eng+deu`. It defaults to `eng`. Converter
limits include 128 MiB per input and output, 500 PDF pages and a five-minute
OCRmyPDF timeout. Safe skips include signed and encrypted PDFs and PDFs with
embedded files. Missing tools, unsuccessful conversion and failed validation
are recorded in the archive attempt.

## Service integration

Use `folio_infra::queue::ArchiveService` after authorizing access to the vault.
Call `status(vault, document)` to get the current attempt for the verified
original and `enqueue(vault, document)` for an initial attempt. Concurrent
initial requests converge on the same attempt ID. To retry a failure or skip
after correcting its cause, call `retry(vault, document, expected_attempt)`
with the ID observed in status. A concurrent update returns a conflict.
`backfill(vault, limit)` admits up to 32 supported originals without an
attempt per call. Repeat until there are no more to enqueue.

Admission commits the attempt and its queue job in one transaction. The job
identifies the vault, revision and attempt. Only the worker that reserves the
artifact key can publish it, and only after verifying the source checksum and
validating the output. Startup and periodic repair settle abandoned
deliveries and remove uncommitted objects after checking whether publication
committed. A retained copy stays under the document prefix until permanent
deletion. Purge waits for running archive jobs and removes the full prefix.
An interrupted or unacknowledged storage write can appear after purge. Folio
keeps a cleanup record outside the document's database lifecycle and
periodically deletes any late object at that key. An absent key or successful
delete does not settle a pending remote write. Cleanup records for
unacknowledged writes remain eligible for recurring checks because the store
cannot report whether the write has finished.
