CREATE UNIQUE INDEX metadata_documents_vault_id ON documents(vault_id, id);
CREATE UNIQUE INDEX metadata_fields_vault_id_unique ON metadata_fields(vault_id, id, is_unique);

CREATE TABLE document_metadata_revisions (
    vault_id UUID NOT NULL,
    document_id UUID NOT NULL,
    revision UUID NOT NULL,
    PRIMARY KEY (vault_id, document_id),
    FOREIGN KEY (vault_id, document_id) REFERENCES documents(vault_id, id) ON DELETE CASCADE
);

CREATE TABLE metadata_values (
    vault_id UUID NOT NULL,
    document_id UUID NOT NULL,
    field_id UUID NOT NULL,
    kind TEXT NOT NULL,
    is_unique BOOLEAN NOT NULL DEFAULT FALSE,
    text_value TEXT,
    boolean_value BOOLEAN,
    integer_value BIGINT,
    decimal_value TEXT,
    date_value TEXT,
    currency TEXT,
    option_id UUID,
    target_document_id UUID,
    canonical_key TEXT,
    created_at TIMESTAMPTZ NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL,
    deleted_at TIMESTAMPTZ,
    created_by TEXT,
    updated_by TEXT,
    PRIMARY KEY (vault_id, document_id, field_id),
    FOREIGN KEY (vault_id, document_id) REFERENCES documents(vault_id, id) ON DELETE CASCADE,
    FOREIGN KEY (vault_id, field_id, kind) REFERENCES metadata_fields(vault_id, id, kind),
    FOREIGN KEY (vault_id, field_id, is_unique) REFERENCES metadata_fields(vault_id, id, is_unique) ON UPDATE CASCADE,
    FOREIGN KEY (vault_id, field_id, option_id) REFERENCES metadata_options(vault_id, field_id, id),
    FOREIGN KEY (vault_id, target_document_id) REFERENCES documents(vault_id, id),
    CHECK (
        (deleted_at IS NOT NULL AND canonical_key IS NULL AND (CASE WHEN text_value IS NULL THEN 0 ELSE 1 END + CASE WHEN boolean_value IS NULL THEN 0 ELSE 1 END + CASE WHEN integer_value IS NULL THEN 0 ELSE 1 END + CASE WHEN decimal_value IS NULL THEN 0 ELSE 1 END + CASE WHEN date_value IS NULL THEN 0 ELSE 1 END + CASE WHEN currency IS NULL THEN 0 ELSE 1 END + CASE WHEN option_id IS NULL THEN 0 ELSE 1 END + CASE WHEN target_document_id IS NULL THEN 0 ELSE 1 END) = 0)
        OR
        (deleted_at IS NULL AND canonical_key IS NOT NULL AND length(canonical_key) = 64 AND (
            (kind = 'text' AND text_value IS NOT NULL AND length(text_value) <= 1024 AND (CASE WHEN text_value IS NULL THEN 0 ELSE 1 END + CASE WHEN boolean_value IS NULL THEN 0 ELSE 1 END + CASE WHEN integer_value IS NULL THEN 0 ELSE 1 END + CASE WHEN decimal_value IS NULL THEN 0 ELSE 1 END + CASE WHEN date_value IS NULL THEN 0 ELSE 1 END + CASE WHEN currency IS NULL THEN 0 ELSE 1 END + CASE WHEN option_id IS NULL THEN 0 ELSE 1 END + CASE WHEN target_document_id IS NULL THEN 0 ELSE 1 END) = 1)
            OR (kind = 'long_text' AND text_value IS NOT NULL AND length(text_value) <= 65536 AND (CASE WHEN text_value IS NULL THEN 0 ELSE 1 END + CASE WHEN boolean_value IS NULL THEN 0 ELSE 1 END + CASE WHEN integer_value IS NULL THEN 0 ELSE 1 END + CASE WHEN decimal_value IS NULL THEN 0 ELSE 1 END + CASE WHEN date_value IS NULL THEN 0 ELSE 1 END + CASE WHEN currency IS NULL THEN 0 ELSE 1 END + CASE WHEN option_id IS NULL THEN 0 ELSE 1 END + CASE WHEN target_document_id IS NULL THEN 0 ELSE 1 END) = 1)
            OR (kind = 'url' AND text_value IS NOT NULL AND length(text_value) <= 8192 AND (CASE WHEN text_value IS NULL THEN 0 ELSE 1 END + CASE WHEN boolean_value IS NULL THEN 0 ELSE 1 END + CASE WHEN integer_value IS NULL THEN 0 ELSE 1 END + CASE WHEN decimal_value IS NULL THEN 0 ELSE 1 END + CASE WHEN date_value IS NULL THEN 0 ELSE 1 END + CASE WHEN currency IS NULL THEN 0 ELSE 1 END + CASE WHEN option_id IS NULL THEN 0 ELSE 1 END + CASE WHEN target_document_id IS NULL THEN 0 ELSE 1 END) = 1)
            OR (kind = 'boolean' AND boolean_value IS NOT NULL AND TRUE AND (CASE WHEN text_value IS NULL THEN 0 ELSE 1 END + CASE WHEN boolean_value IS NULL THEN 0 ELSE 1 END + CASE WHEN integer_value IS NULL THEN 0 ELSE 1 END + CASE WHEN decimal_value IS NULL THEN 0 ELSE 1 END + CASE WHEN date_value IS NULL THEN 0 ELSE 1 END + CASE WHEN currency IS NULL THEN 0 ELSE 1 END + CASE WHEN option_id IS NULL THEN 0 ELSE 1 END + CASE WHEN target_document_id IS NULL THEN 0 ELSE 1 END) = 1)
            OR (kind = 'integer' AND integer_value IS NOT NULL AND TRUE AND (CASE WHEN text_value IS NULL THEN 0 ELSE 1 END + CASE WHEN boolean_value IS NULL THEN 0 ELSE 1 END + CASE WHEN integer_value IS NULL THEN 0 ELSE 1 END + CASE WHEN decimal_value IS NULL THEN 0 ELSE 1 END + CASE WHEN date_value IS NULL THEN 0 ELSE 1 END + CASE WHEN currency IS NULL THEN 0 ELSE 1 END + CASE WHEN option_id IS NULL THEN 0 ELSE 1 END + CASE WHEN target_document_id IS NULL THEN 0 ELSE 1 END) = 1)
            OR (kind = 'decimal' AND decimal_value IS NOT NULL AND length(decimal_value) <= 40 AND (CASE WHEN text_value IS NULL THEN 0 ELSE 1 END + CASE WHEN boolean_value IS NULL THEN 0 ELSE 1 END + CASE WHEN integer_value IS NULL THEN 0 ELSE 1 END + CASE WHEN decimal_value IS NULL THEN 0 ELSE 1 END + CASE WHEN date_value IS NULL THEN 0 ELSE 1 END + CASE WHEN currency IS NULL THEN 0 ELSE 1 END + CASE WHEN option_id IS NULL THEN 0 ELSE 1 END + CASE WHEN target_document_id IS NULL THEN 0 ELSE 1 END) = 1)
            OR (kind = 'date' AND date_value IS NOT NULL AND length(date_value) = 10 AND (CASE WHEN text_value IS NULL THEN 0 ELSE 1 END + CASE WHEN boolean_value IS NULL THEN 0 ELSE 1 END + CASE WHEN integer_value IS NULL THEN 0 ELSE 1 END + CASE WHEN decimal_value IS NULL THEN 0 ELSE 1 END + CASE WHEN date_value IS NULL THEN 0 ELSE 1 END + CASE WHEN currency IS NULL THEN 0 ELSE 1 END + CASE WHEN option_id IS NULL THEN 0 ELSE 1 END + CASE WHEN target_document_id IS NULL THEN 0 ELSE 1 END) = 1)
            OR (kind = 'money' AND decimal_value IS NOT NULL AND length(decimal_value) <= 40 AND currency IS NOT NULL AND length(currency) = 3 AND (CASE WHEN text_value IS NULL THEN 0 ELSE 1 END + CASE WHEN boolean_value IS NULL THEN 0 ELSE 1 END + CASE WHEN integer_value IS NULL THEN 0 ELSE 1 END + CASE WHEN decimal_value IS NULL THEN 0 ELSE 1 END + CASE WHEN date_value IS NULL THEN 0 ELSE 1 END + CASE WHEN currency IS NULL THEN 0 ELSE 1 END + CASE WHEN option_id IS NULL THEN 0 ELSE 1 END + CASE WHEN target_document_id IS NULL THEN 0 ELSE 1 END) = 2)
            OR (kind = 'select' AND option_id IS NOT NULL AND (CASE WHEN text_value IS NULL THEN 0 ELSE 1 END + CASE WHEN boolean_value IS NULL THEN 0 ELSE 1 END + CASE WHEN integer_value IS NULL THEN 0 ELSE 1 END + CASE WHEN decimal_value IS NULL THEN 0 ELSE 1 END + CASE WHEN date_value IS NULL THEN 0 ELSE 1 END + CASE WHEN currency IS NULL THEN 0 ELSE 1 END + CASE WHEN option_id IS NULL THEN 0 ELSE 1 END + CASE WHEN target_document_id IS NULL THEN 0 ELSE 1 END) = 1)
            OR (kind = 'document_link' AND target_document_id IS NOT NULL AND (CASE WHEN text_value IS NULL THEN 0 ELSE 1 END + CASE WHEN boolean_value IS NULL THEN 0 ELSE 1 END + CASE WHEN integer_value IS NULL THEN 0 ELSE 1 END + CASE WHEN decimal_value IS NULL THEN 0 ELSE 1 END + CASE WHEN date_value IS NULL THEN 0 ELSE 1 END + CASE WHEN currency IS NULL THEN 0 ELSE 1 END + CASE WHEN option_id IS NULL THEN 0 ELSE 1 END + CASE WHEN target_document_id IS NULL THEN 0 ELSE 1 END) = 1)
        ))
    )
);
CREATE UNIQUE INDEX metadata_values_unique_value ON metadata_values(vault_id, field_id, canonical_key) WHERE is_unique AND deleted_at IS NULL;
CREATE INDEX metadata_values_target ON metadata_values(vault_id, target_document_id) WHERE target_document_id IS NOT NULL;
CREATE INDEX metadata_values_exact ON metadata_values(vault_id, field_id, canonical_key) WHERE deleted_at IS NULL;
