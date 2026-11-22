## Why

A committed event can carry both a domain occurrence and a legacy document projection. The dispatcher currently chooses only one path. Clients also need the same selectable trigger names and labels.

## What changes

Resolve registered candidates from both representations. Dispatch each selected trigger with its own payload and preserve database deduplication. Expose the shared trigger choices and occurrence fields to authorized builders.

## Impact

Changes folio-core dispatch and the folio-api web editor and JSON catalog. Regenerates OpenAPI for the generated Swift client. No storage migration or new configuration.
