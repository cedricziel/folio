## Identity

Source item keys hash length-prefixed components with a kind/version prefix. Persistence scopes these keys by source and vault; they never replace content deduplication keys.

Folder keys use the relative path, persisted staging directory name, and bytes. A recovered staged file retains its key. A new arrival gets a new staging generation even if its path and bytes match an older arrival.

Email keys use mailbox, UIDVALIDITY, UID, and attachment position before configuration filtering. The optional whole-message item uses position zero; attachments start at one. Message-ID remains the fallback on servers without usable UIDVALIDITY, preserving existing ingestion eligibility. Mailbox cleanup and message ledger keys remain unchanged.

## Compatibility

`IngestSink::ingest_observed` and `VaultIngest::ingest_observed` default to their existing ingest methods. The scheduler forwards the key across the vault binding. Existing sink implementations retain their behavior while outcome-aware consumers can use source item identity. The adapters do not publish an event or change pre-import veto semantics.

The existing transactional outbox and its foundation-owned domain envelope remain the only publication mechanism. This increment does not depend on those unmerged APIs.
