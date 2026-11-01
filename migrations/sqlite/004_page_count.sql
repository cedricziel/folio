-- Add page_count column to documents table for multi-page support
ALTER TABLE documents ADD COLUMN page_count INTEGER;
