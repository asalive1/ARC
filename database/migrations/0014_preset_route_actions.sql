ALTER TABLE preset_routes
    ADD COLUMN action TEXT NOT NULL DEFAULT 'start'
    CHECK (action IN ('start', 'stop'));
