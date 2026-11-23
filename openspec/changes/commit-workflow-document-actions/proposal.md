## Why

A workflow can commit a document mutation and stop before saving its completed action count. A restart can then repeat that action and overwrite a subsequent manual edit.

## What changes

Commit each local mutation, its outbox event, and the run’s progress in one transaction. Support rename, tag removal, classification clearing, setting and clearing the document date, review, and moving a document to Trash. Keep Trash as the final action because later steps require an active document.

## Impact

SQLite and PostgreSQL use the existing completed-action column. The runner reads the captured action from the locked run and checks the active workflow and vault before applying it. Existing webhook deliveries keep their retry and delivery identity behavior. The workflow API and generated Swift client gain the new action types.
