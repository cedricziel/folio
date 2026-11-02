## Why

Domain producers need one way to declare workflow triggers and their compatible actions. A document ID in an event payload does not by itself guarantee a mutable document target.

## What changes

Add a typed trigger registry with explicit document, event-only, and ingestion-candidate targets. Reject duplicate producer registrations, unknown or repeated selections, malformed condition catalogs, and incompatible action combinations. Reuse target compatibility in legacy workflow validation.

## Impact

Touches folio-core models only. No HTTP endpoint, persistence, settings, or event publication changes. Producers and shared web/Apple builders will integrate this catalog in later increments.
