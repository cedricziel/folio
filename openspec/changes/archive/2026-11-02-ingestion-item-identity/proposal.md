## Why

Content hashes identify duplicate documents but cannot distinguish source items or preserve their identity across retries. Durable outcome recording needs that distinction before it can suppress repeated observations.

## What changes

- Give folder and email observations opaque keys independent of content deduplication.
- Preserve folder staging identity across restart and distinguish a same-path replacement.
- Use mailbox UIDVALIDITY, UID, and attachment position before filtering for email identity, with the existing Message-ID fallback when UIDVALIDITY is unavailable.
- Forward keys through source and vault sinks without changing existing import results or mail cleanup.

## Impact

Touches folio-core models/ports and scheduler forwarding, folio-infra folder/email adapters, and their regression tests. No database migration, HTTP route, setting, event envelope, or UI change. Outcome persistence and publication remain separate work using the shared foundation publisher.
