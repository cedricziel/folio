# Archive workflows

## Find and read evidence

Call `search_documents` with words or metadata filters. Every query word must
match; omit `query` for metadata-only searches. Results default to 10 and are
limited to 50. Narrow broad searches rather than claiming the results cover the
whole archive. Dates filter when documents were added, not invoice or issue dates.

Read candidate documents with `get_document` using their returned IDs. Pass
`next_cursor` back as `cursor` with the same ID to continue; do not invent cursors.
The default page has 8,000 characters and `max_chars` allows up to 50,000.
Distinguish missing OCR from evidence that a document lacks the requested fact.
Check `get_processing_status` if text is unavailable. State uncertainty if
processing is incomplete or failed.

Cite titles and `resource_uri` values. Use `folio://documents/{id}/file` only when
the original is needed; this resource returns at most 5 MiB. Do not infer a public
HTTP download URL from a resource URI.

## Organize documents

Identify the intended documents with search and read their current metadata.
Resolve ambiguous matches before changing them. Inspect the existing vocabulary
with `list_tags`, `list_correspondents`, and `list_document_types`; names are
case-insensitive.

Use `add_tags` and `remove_tags` for incremental tag changes. `update_document`
replaces all tags when `tags` is supplied. Omit fields that should stay unchanged.
An empty correspondent or document type clears it; a new name creates it.
Apply only the requested edits and report the returned metadata. There is no
MCP delete tool.

## Upload and check processing

Call `upload_document` with `filename` and `content_base64`, plus optional `title`
and `mime_type`. Use the actual file bytes, at most 25 MiB decoded. Never invent
file contents or send a local path in place of base64 data.

Keep the returned ID. Upload completion means the file was accepted; OCR and
indexing run asynchronously. Check `get_processing_status` for that ID as needed,
without a tight polling loop. Report pending or failed processing honestly.
Do not upload again just because processing is pending. If the upload response
was lost, search for the document before retrying to avoid duplicates.

After processing completes, use `get_document` to read the text and apply any
metadata changes the user requested.
