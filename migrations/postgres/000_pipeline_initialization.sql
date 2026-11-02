CREATE TABLE pipeline_initialization (
    id INTEGER PRIMARY KEY CHECK (id = 1),
    pending BOOLEAN NOT NULL
);

INSERT INTO pipeline_initialization (id, pending)
SELECT 1, NOT EXISTS (SELECT 1 FROM _sqlx_migrations);
