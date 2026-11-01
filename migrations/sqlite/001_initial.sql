-- Folio Initial Schema
-- Created for SQLx migrations

-- Correspondents (document senders)
CREATE TABLE IF NOT EXISTS correspondents (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL UNIQUE,
    created_at TEXT NOT NULL
);

-- Document types (classification)
CREATE TABLE IF NOT EXISTS document_types (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL UNIQUE,
    created_at TEXT NOT NULL
);

-- Documents
CREATE TABLE IF NOT EXISTS documents (
    id TEXT PRIMARY KEY,
    title TEXT NOT NULL,
    content TEXT,
    original_filename TEXT,
    file_path TEXT,
    mime_type TEXT,
    correspondent_id TEXT REFERENCES correspondents(id) ON DELETE SET NULL,
    document_type_id TEXT REFERENCES document_types(id) ON DELETE SET NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

-- Tags
CREATE TABLE IF NOT EXISTS tags (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL UNIQUE,
    color TEXT,
    created_at TEXT NOT NULL
);

-- Document-Tag association
CREATE TABLE IF NOT EXISTS document_tags (
    document_id TEXT REFERENCES documents(id) ON DELETE CASCADE,
    tag_id TEXT REFERENCES tags(id) ON DELETE CASCADE,
    PRIMARY KEY (document_id, tag_id)
);

-- Document embeddings for semantic search
CREATE TABLE IF NOT EXISTS document_embeddings (
    document_id TEXT PRIMARY KEY REFERENCES documents(id) ON DELETE CASCADE,
    embedding TEXT NOT NULL
);

-- Indexes for common queries
CREATE INDEX IF NOT EXISTS idx_documents_correspondent ON documents(correspondent_id);
CREATE INDEX IF NOT EXISTS idx_documents_type ON documents(document_type_id);
CREATE INDEX IF NOT EXISTS idx_documents_created ON documents(created_at);
CREATE INDEX IF NOT EXISTS idx_tags_name ON tags(name);
CREATE INDEX IF NOT EXISTS idx_correspondents_name ON correspondents(name);
CREATE INDEX IF NOT EXISTS idx_document_types_name ON document_types(name);
