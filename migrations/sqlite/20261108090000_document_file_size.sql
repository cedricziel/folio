-- Size of each stored original in bytes; NULL until recorded or backfilled

ALTER TABLE documents ADD COLUMN file_size INTEGER CHECK (file_size >= 0);
