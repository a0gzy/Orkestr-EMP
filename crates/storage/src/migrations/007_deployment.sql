CREATE TABLE managed_installers(
    revision_id TEXT PRIMARY KEY REFERENCES package_revisions(id),
    data TEXT NOT NULL
);
CREATE TABLE deployments(
    id TEXT PRIMARY KEY,
    actor_id TEXT NOT NULL,
    idempotency TEXT NOT NULL,
    device_id TEXT NOT NULL REFERENCES devices(id),
    revision INTEGER NOT NULL,
    holds_device INTEGER NOT NULL CHECK(holds_device IN (0,1)),
    data TEXT NOT NULL,
    UNIQUE(actor_id,idempotency)
);
CREATE UNIQUE INDEX deployments_device_lock ON deployments(device_id) WHERE holds_device=1;
