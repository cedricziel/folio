## Context

Move currently copies a message and shares Delete's unrestricted expunge. Other clients can mark unrelated messages deleted between commands, so temporarily clearing their flags cannot make mailbox-wide expunge safe.

## Decisions

- Read authenticated server capabilities before destructive cleanup. Mark Read continues to use only `UID STORE`.
- Use `UID MOVE` when MOVE is advertised. Otherwise, verify UIDPLUS before issuing COPY or changing flags, then use `UID STORE` and `UID EXPUNGE` with the imported UID.
- Without the required capabilities, return a cleanup error before mutation. Existing polling logs the failure and retains the successful import ledger entry, so later polls can retry cleanup without importing the document again.
- A rejected scoped command returns an error. Never retry with mailbox-wide EXPUNGE or CLOSE.
- Test both actions against disposable GreenMail mailboxes containing an unrelated pre-deleted message. A loopback IMAP fixture exercises capability combinations and rejected commands independently of Docker.

## Risks

A server that lacks UIDPLUS cannot complete Delete. A server that lacks both MOVE and UIDPLUS cannot complete Move. Messages remain in the source mailbox, and users can choose Mark Read or clean up manually. A network failure after COPY can still leave a duplicate destination copy on retry, matching the existing non-atomic fallback behavior.
