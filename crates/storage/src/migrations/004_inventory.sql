CREATE TABLE inventory_reports(
    id TEXT PRIMARY KEY,
    device_id TEXT NOT NULL REFERENCES devices(id),
    started_seconds INTEGER NOT NULL,
    started_nanos INTEGER NOT NULL,
    finished_seconds INTEGER NOT NULL,
    finished_nanos INTEGER NOT NULL,
    scan_id TEXT UNIQUE REFERENCES scans(id),
    data TEXT NOT NULL
);
CREATE INDEX inventory_reports_device ON inventory_reports(device_id,
    started_seconds DESC, started_nanos DESC, finished_seconds DESC, finished_nanos DESC, id DESC);
