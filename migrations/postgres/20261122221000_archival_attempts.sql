CREATE TABLE archival_attempts (
    id UUID PRIMARY KEY NOT NULL,
    document_id UUID NOT NULL,
    revision_id UUID NOT NULL,
    source_sha256 TEXT NOT NULL CHECK (source_sha256 ~ '^[0-9a-f]{64}$'),
    is_current BOOLEAN NOT NULL DEFAULT TRUE,
    state TEXT NOT NULL DEFAULT 'queued',
    reason TEXT,
    output_storage_key TEXT UNIQUE,
    output_sha256 TEXT,
    output_size BIGINT,
    profile TEXT,
    converter_name TEXT,
    converter_version TEXT,
    validator_name TEXT,
    validator_version TEXT,
    FOREIGN KEY (revision_id, document_id) REFERENCES document_revisions (id, document_id)
        ON DELETE CASCADE,
    CHECK (state IN ('queued', 'ready', 'skipped', 'failed')),
    CHECK (
        (state IN ('skipped', 'failed') AND reason IS NOT NULL AND length(trim(reason)) > 0)
        OR (state IN ('queued', 'ready') AND reason IS NULL)
    ),
    CHECK (
        (state = 'ready' AND output_storage_key IS NOT NULL AND length(output_storage_key) > 0
            AND output_sha256 IS NOT NULL AND output_sha256 ~ '^[0-9a-f]{64}$'
            AND output_size IS NOT NULL AND output_size > 0
            AND profile IS NOT NULL AND length(trim(profile)) > 0
            AND converter_name IS NOT NULL AND length(trim(converter_name)) > 0
            AND converter_version IS NOT NULL AND length(trim(converter_version)) > 0
            AND validator_name IS NOT NULL AND length(trim(validator_name)) > 0
            AND validator_version IS NOT NULL AND length(trim(validator_version)) > 0)
        OR (state <> 'ready' AND output_storage_key IS NULL AND output_sha256 IS NULL AND output_size IS NULL AND profile IS NULL
            AND converter_name IS NULL AND converter_version IS NULL
            AND validator_name IS NULL AND validator_version IS NULL)
    )
);

CREATE UNIQUE INDEX archival_attempts_current ON archival_attempts (revision_id) WHERE is_current;
CREATE INDEX archival_attempts_document ON archival_attempts (document_id, revision_id);
