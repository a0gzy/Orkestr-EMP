CREATE TABLE server_identity(id INTEGER PRIMARY KEY CHECK(id=1), owner_sid TEXT NOT NULL, canary BLOB NOT NULL);
CREATE TABLE operators(id TEXT PRIMARY KEY, token_hash TEXT UNIQUE NOT NULL, data TEXT NOT NULL, revoked INTEGER NOT NULL DEFAULT 0);
CREATE TABLE credential_profiles(id TEXT PRIMARY KEY, revision INTEGER NOT NULL, data TEXT NOT NULL, ciphertext BLOB NOT NULL);
CREATE TABLE connection_profiles(id TEXT PRIMARY KEY, revision INTEGER NOT NULL, data TEXT NOT NULL);
CREATE TABLE server_jobs(id TEXT PRIMARY KEY, actor_id TEXT NOT NULL, idempotency TEXT NOT NULL, data TEXT NOT NULL, connection_revision INTEGER NOT NULL, credential_revision INTEGER NOT NULL, UNIQUE(actor_id,idempotency));
CREATE TABLE audit_events(cursor INTEGER PRIMARY KEY AUTOINCREMENT, at TEXT NOT NULL, actor TEXT NOT NULL, verb TEXT NOT NULL, target TEXT NOT NULL, code TEXT NOT NULL);
