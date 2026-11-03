ALTER TABLE documents ADD COLUMN document_date TEXT
    CHECK (document_date IS NULL OR (
        length(document_date) = 10
        AND document_date GLOB '[0-9][0-9][0-9][0-9]-[0-9][0-9]-[0-9][0-9]'
        AND document_date BETWEEN '0001-01-01' AND '9999-12-31'
        AND date(document_date, '+0 days') IS document_date
    ));
