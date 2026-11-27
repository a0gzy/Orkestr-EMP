CREATE TABLE metadata (id INTEGER PRIMARY KEY CHECK(id=1), generation INTEGER NOT NULL);
INSERT INTO metadata VALUES (1, 0);
CREATE TABLE devices (
    id TEXT PRIMARY KEY,
    host TEXT NOT NULL UNIQUE COLLATE NOCASE,
    ad_guid TEXT UNIQUE,
    data TEXT NOT NULL
);
CREATE TABLE device_aliases (
    host TEXT PRIMARY KEY COLLATE NOCASE,
    device_id TEXT NOT NULL REFERENCES devices(id) ON DELETE RESTRICT
);
CREATE TABLE scans (
    id TEXT PRIMARY KEY,
    device_id TEXT NOT NULL REFERENCES devices(id) ON DELETE RESTRICT,
    scanned_at TEXT NOT NULL,
    data TEXT NOT NULL
);
CREATE INDEX ix_scans_device ON scans(device_id, scanned_at);
CREATE TABLE scan_heads (
    device_id TEXT PRIMARY KEY REFERENCES devices(id),
    scan_id TEXT NOT NULL UNIQUE REFERENCES scans(id)
);
CREATE TABLE installed_software (
    scan_id TEXT NOT NULL REFERENCES scans(id),
    ordinal INTEGER NOT NULL,
    application_id TEXT,
    name TEXT NOT NULL,
    publisher TEXT NOT NULL,
    version TEXT NOT NULL,
    channel TEXT,
    architecture TEXT,
    PRIMARY KEY(scan_id, ordinal)
);
CREATE TABLE collections (
    id TEXT PRIMARY KEY,
    parent_id TEXT REFERENCES collections(id) ON DELETE RESTRICT,
    revision INTEGER NOT NULL CHECK(revision>=1),
    data TEXT NOT NULL
);
CREATE INDEX ix_collection_parent ON collections(parent_id);
CREATE TABLE package_revisions (
    id TEXT PRIMARY KEY,
    package_id TEXT NOT NULL,
    data TEXT NOT NULL
);
CREATE TABLE imports (
    source_hash TEXT PRIMARY KEY,
    format TEXT NOT NULL,
    imported_at TEXT NOT NULL,
    report TEXT NOT NULL
);
CREATE TABLE legacy_ids (
    source_hash TEXT NOT NULL REFERENCES imports(source_hash),
    entity TEXT NOT NULL,
    legacy_key TEXT NOT NULL,
    new_id TEXT NOT NULL,
    PRIMARY KEY(source_hash, entity, legacy_key)
);
