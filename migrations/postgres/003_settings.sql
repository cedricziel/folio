-- Application settings stored in database

CREATE TABLE settings (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL,
    description TEXT,
    updated_at TIMESTAMPTZ NOT NULL
);

INSERT INTO settings (key, value, description, updated_at) VALUES
    ('ai.provider', 'local', 'AI provider: openai, anthropic, or local', now()),
    ('ai.openai_api_key', '', 'OpenAI API key for OCR and embeddings', now()),
    ('ai.anthropic_api_key', '', 'Anthropic API key for OCR', now()),
    ('ai.ocr_model', 'gpt-4o-mini', 'Model to use for OCR', now()),
    ('ai.embedding_model', 'text-embedding-3-small', 'Model to use for embeddings', now()),
    ('storage.path', './data', 'Path for document storage', now()),
    ('processing.auto_ocr', 'true', 'Automatically run OCR on uploaded documents', now()),
    ('processing.auto_thumbnail', 'true', 'Automatically generate thumbnails', now()),
    ('processing.auto_embedding', 'true', 'Automatically generate embeddings for search', now());
