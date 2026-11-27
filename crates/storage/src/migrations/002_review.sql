-- Quarantined, NON-executable migration metadata. There is no schedule/job table.
CREATE TABLE import_review (
    id TEXT PRIMARY KEY,
    source_hash TEXT NOT NULL REFERENCES imports(source_hash),
    kind TEXT NOT NULL,
    name TEXT NOT NULL,
    data TEXT NOT NULL
);
CREATE TABLE package_lineage (
    origin_key TEXT NOT NULL,
    legacy_key TEXT NOT NULL,
    package_id TEXT NOT NULL,
    PRIMARY KEY(origin_key, legacy_key)
);
