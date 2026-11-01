-- Application settings stored in database

CREATE TABLE settings (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL,
    description TEXT,
    updated_at TEXT NOT NULL
);

-- Insert default settings
INSERT INTO settings (key, value, description, updated_at) VALUES
    ('ai.provider', 'local', 'AI provider: openai, anthropic, or local', datetime('now')),
    ('ai.openai_api_key', '', 'OpenAI API key for OCR and embeddings', datetime('now')),
    ('ai.anthropic_api_key', '', 'Anthropic API key for OCR', datetime('now')),
    ('ai.ocr_model', 'gpt-4o-mini', 'Model to use for OCR', datetime('now')),
    ('ai.embedding_model', 'text-embedding-3-small', 'Model to use for embeddings', datetime('now')),
    ('storage.path', './data', 'Path for document storage', datetime('now')),
    ('processing.auto_ocr', 'true', 'Automatically run OCR on uploaded documents', datetime('now')),
    ('processing.auto_thumbnail', 'true', 'Automatically generate thumbnails', datetime('now')),
    ('processing.auto_embedding', 'true', 'Automatically generate embeddings for search', datetime('now'));
