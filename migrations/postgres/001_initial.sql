-- Folio initial schema (PostgreSQL)

CREATE TABLE IF NOT EXISTS correspondents (
    id UUID PRIMARY KEY,
    name TEXT NOT NULL UNIQUE,
    created_at TIMESTAMPTZ NOT NULL
);

CREATE TABLE IF NOT EXISTS document_types (
    id UUID PRIMARY KEY,
    name TEXT NOT NULL UNIQUE,
    created_at TIMESTAMPTZ NOT NULL
);

CREATE TABLE IF NOT EXISTS documents (
    id UUID PRIMARY KEY,
    title TEXT NOT NULL,
    content TEXT,
    original_filename TEXT,
    file_path TEXT,
    mime_type TEXT,
    correspondent_id UUID REFERENCES correspondents(id) ON DELETE SET NULL,
    document_type_id UUID REFERENCES document_types(id) ON DELETE SET NULL,
    created_at TIMESTAMPTZ NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL
);

CREATE TABLE IF NOT EXISTS tags (
    id UUID PRIMARY KEY,
    name TEXT NOT NULL UNIQUE,
    color TEXT,
    created_at TIMESTAMPTZ NOT NULL
);

CREATE TABLE IF NOT EXISTS document_tags (
    document_id UUID REFERENCES documents(id) ON DELETE CASCADE,
    tag_id UUID REFERENCES tags(id) ON DELETE CASCADE,
    PRIMARY KEY (document_id, tag_id)
);

-- Embeddings are always kept as real[] so cosine search works without
-- extensions. When pgvector is installed the app adds an `embedding_vec`
-- column at startup and uses it for ranking.
CREATE TABLE IF NOT EXISTS document_embeddings (
    document_id UUID PRIMARY KEY REFERENCES documents(id) ON DELETE CASCADE,
    embedding REAL[] NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_documents_correspondent ON documents(correspondent_id);
CREATE INDEX IF NOT EXISTS idx_documents_type ON documents(document_type_id);
CREATE INDEX IF NOT EXISTS idx_documents_created ON documents(created_at);
