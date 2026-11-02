# Document trash and recoverable permanent deletion

## Why
Default deletion currently removes files before database state changes. A database failure can leave a visible document without its original. Users also need to recover accidental deletions (#91, #131).

## What changes
Default deletion moves documents to Trash. Documents remain there until explicitly deleted forever; there is no automatic retention period. Restore keeps metadata and files. Permanent deletion records durable intent before a worker removes storage and then database state.

This touches folio-core models and repository ports, folio-infra database adapters and workers, folio-api JSON and HTML routes, web templates, and Apple deletion wording. No new configuration setting is introduced.

Migration `20261102130000_document_trash` adds nullable deletion timestamps in SQLite and PostgreSQL. Existing rows remain active. Existing migrations are preserved.
