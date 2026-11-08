-- Size of each stored original in bytes; NULL until recorded or backfilled

ALTER TABLE documents ADD COLUMN file_size BIGINT CHECK (file_size >= 0);
