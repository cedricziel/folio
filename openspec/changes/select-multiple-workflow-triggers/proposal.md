## Why

Vault owners need one workflow to respond to several committed events while keeping one ordered action list and one run per originating mutation.

## What changes

Accept an ordered trigger list and retain the legacy single-trigger field for existing clients. Validate each action and event condition against every selected trigger. Match both primary and additional triggers through the existing revision and mutation fences.

## Impact

SQLite and PostgreSQL add `20261123000100_workflow_additional_triggers.sql`. Existing configurations keep their primary trigger and gain an empty list of additional triggers. An older client can update a workflow with its unchanged primary trigger and preserve its additional choices. The OpenAPI contract describes both request shapes.
