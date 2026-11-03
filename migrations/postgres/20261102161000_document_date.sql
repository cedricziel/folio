ALTER TABLE documents ADD COLUMN document_date DATE
    CHECK (document_date BETWEEN DATE '0001-01-01' AND DATE '9999-12-31');
