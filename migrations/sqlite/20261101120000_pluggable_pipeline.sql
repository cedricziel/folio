-- Pluggable processing pipeline: per-stage status, chunked model-tagged
-- embeddings, and provider settings per stage.

-- Per-stage status so "no text found" is distinguishable from "not processed yet".
ALTER TABLE documents ADD COLUMN ocr_status TEXT NOT NULL DEFAULT 'pending';
ALTER TABLE documents ADD COLUMN embed_status TEXT NOT NULL DEFAULT 'pending';

UPDATE documents SET ocr_status = 'done' WHERE content IS NOT NULL;
-- Old single-vector embeddings carry no model id and cannot be searched
-- safely, so every document with text is re-embedded under the active model.
UPDATE documents SET embed_status = 'skipped' WHERE content IS NULL OR content = '';

DROP TABLE document_embeddings;

CREATE TABLE document_chunks (
    document_id TEXT NOT NULL REFERENCES documents(id) ON DELETE CASCADE,
    chunk_index INTEGER NOT NULL,
    text TEXT NOT NULL,
    model TEXT NOT NULL,
    dimensions INTEGER NOT NULL,
    embedding BLOB NOT NULL,
    stale INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL,
    PRIMARY KEY (document_id, chunk_index)
);

CREATE INDEX idx_document_chunks_model ON document_chunks(model, stale);

-- Per-stage provider settings, seeded from the legacy ai.* keys.
INSERT OR IGNORE INTO settings (key, value, description, updated_at) VALUES
    ('ocr.provider',
     CASE (SELECT value FROM settings WHERE key = 'ai.provider')
         WHEN 'openai' THEN 'openai-vision'
         WHEN 'anthropic' THEN 'anthropic-vision'
         ELSE 'tesseract'
     END,
     'OCR provider: tesseract, openai-vision, anthropic-vision, ollama-vision, none',
     strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    ('ocr.model',
     CASE (SELECT value FROM settings WHERE key = 'ai.provider')
         WHEN 'openai' THEN COALESCE((SELECT value FROM settings WHERE key = 'ai.ocr_model'), '')
         WHEN 'anthropic' THEN COALESCE(
             (SELECT value FROM settings WHERE key = 'ai.ocr_model' AND value LIKE 'claude%'), '')
         ELSE ''
     END,
     'OCR model for vision providers (empty uses the provider default)',
     strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    ('ocr.languages', 'eng', 'OCR languages, tesseract style (e.g. deu+eng)',
     strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    ('ocr.base_url', '', 'Endpoint override for Ollama or OpenAI-compatible OCR servers',
     strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    ('embeddings.provider',
     CASE (SELECT value FROM settings WHERE key = 'ai.provider')
         WHEN 'openai' THEN 'openai'
         ELSE 'none'
     END,
     'Embedding provider: openai, ollama, fastembed, none',
     strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    ('embeddings.model',
     CASE (SELECT value FROM settings WHERE key = 'ai.provider')
         WHEN 'openai' THEN COALESCE((SELECT value FROM settings WHERE key = 'ai.embedding_model'), '')
         ELSE ''
     END,
     'Embedding model (empty uses the provider default)',
     strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    ('embeddings.base_url', '', 'Endpoint override for Ollama or OpenAI-compatible embedding servers',
     strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    ('processing.auto_embed',
     COALESCE((SELECT value FROM settings WHERE key = 'processing.auto_embedding'), 'true'),
     'Automatically generate embeddings for search',
     strftime('%Y-%m-%dT%H:%M:%SZ', 'now'));

DELETE FROM settings WHERE key IN
    ('ai.provider', 'ai.ocr_model', 'ai.embedding_model', 'processing.auto_embedding');
