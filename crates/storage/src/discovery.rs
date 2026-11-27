use crate::{server::audit, store::*, Error, Result, Store};
use chrono::{DateTime, Duration, Utc};
use orkestr_inventory::{DiscoveryBatch, DiscoveryPreview};
use orkestr_protocol::*;
use rusqlite::{params, OptionalExtension, TransactionBehavior};
use uuid::Uuid;

const SESSION_CAPACITY: usize = 8;
pub(crate) struct Session {
    owner: Uuid,
    expires_at: DateTime<Utc>,
    plan: DiscoveryPreview,
}

impl Store {
    pub fn prepare_discovery_session(
        &mut self,
        owner: Uuid,
        input: &PrepareDiscovery,
    ) -> Result<PreparedDiscovery> {
        // A new attempt invalidates this actor's previous draft, even if parsing fails.
        self.discard_discovery_session(owner);
        if input.text.len() > DISCOVERY_TEXT_LIMIT {
            return Err(Error::Invalid("discovery UI/API input exceeds 24 KiB"));
        }
        let batch = match input.format {
            DiscoverySource::Manual => DiscoveryBatch::manual(&input.text),
            DiscoverySource::Csv => DiscoveryBatch::csv(&input.text),
            DiscoverySource::Ad => return Err(Error::Invalid("use the server directory job")),
        }?;
        self.prepare_discovery_batch(owner, &batch)
    }

    pub fn prepare_discovery_batch(
        &mut self,
        owner: Uuid,
        batch: &DiscoveryBatch,
    ) -> Result<PreparedDiscovery> {
        let now = Utc::now();
        self.discovery_sessions
            .retain(|_, s| s.owner != owner && s.expires_at > now);
        if self.discovery_sessions.len() >= SESSION_CAPACITY {
            return Err(Error::DiscoveryBusy);
        }
        let plan = self.preview_discovery(batch)?;
        let id = Uuid::new_v4();
        let expires_at = now + Duration::minutes(5);
        let response = PreparedDiscovery {
            id,
            generation: plan.generation(),
            expires_at,
            rows: plan.rows().to_vec(),
            changes: plan.writes().len(),
            can_apply: plan.can_apply(),
        };
        self.discovery_sessions.insert(
            id,
            Session {
                owner,
                expires_at,
                plan,
            },
        );
        Ok(response)
    }

    pub fn discard_discovery_session(&mut self, owner: Uuid) {
        self.discovery_sessions.retain(|_, s| s.owner != owner);
    }

    /// Actor-bound durable idempotency: replay returns the receipt even after restart.
    /// Backup names are generated here, adjacent to this backend's DB, never from HTTP input.
    pub fn apply_discovery_session(
        &mut self,
        owner: Uuid,
        input: &ApplyDiscovery,
    ) -> Result<AppliedDiscovery> {
        if !input.confirmed {
            return Err(Error::Invalid("explicit discovery confirmation required"));
        }
        let existing: Option<(String, String)> = self
            .db
            .query_row(
                "SELECT owner_id,data FROM discovery_receipts WHERE preview_id=?1",
                [input.preview_id.to_string()],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        if let Some((id, data)) = existing {
            if id != owner.to_string() {
                return Err(Error::PreviewUnavailable);
            }
            return Ok(serde_json::from_str(&data)?);
        }
        let snapshot = self.snapshot()?;
        let session = self
            .discovery_sessions
            .get(&input.preview_id)
            .ok_or(Error::PreviewUnavailable)?;
        if session.owner != owner || session.expires_at <= Utc::now() {
            return Err(Error::PreviewUnavailable);
        }
        if !session.plan.can_apply() {
            return Err(Error::Invalid("discovery preview has conflicts"));
        }
        if !session.plan.is_current(&snapshot) {
            return Err(Error::Conflict);
        }
        let changes = session.plan.writes().len();
        // Close the immutable borrow before SQLite backup / transaction.
        let backup = if changes > 0 {
            let name = format!("before-discovery-{}.sqlite", input.preview_id);
            let database = self
                .db
                .path()
                .ok_or(Error::Invalid("file-backed database required"))?;
            let path = std::path::Path::new(database)
                .parent()
                .ok_or(Error::Invalid("database directory unavailable"))?
                .join(&name);
            // A prior failed/ambiguous attempt may have left a backup. Never overwrite it;
            // a fresh random name makes retries safe without trusting an existing file.
            let (name, path) = if path.exists() {
                let name = format!("before-discovery-{}.sqlite", Uuid::new_v4());
                let path = path.with_file_name(&name);
                (name, path)
            } else {
                (name, path)
            };
            self.backup(&path)?;
            Some(name)
        } else {
            None
        };
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        // Also handles another local handle modifying the DB while VACUUM backup ran.
        let session = self
            .discovery_sessions
            .get(&input.preview_id)
            .ok_or(Error::PreviewUnavailable)?;
        if session.expires_at <= Utc::now() {
            return Err(Error::PreviewUnavailable);
        }
        if !session.plan.is_current(&load_snapshot(&tx)?) {
            return Err(Error::Conflict);
        }
        for device in session.plan.writes() {
            write_device(&tx, device)?;
        }
        if changes > 0 {
            bump(&tx)?;
        }
        let generation = tx.query_row("SELECT generation FROM metadata WHERE id=1", [], |r| {
            r.get(0)
        })?;
        let result = AppliedDiscovery {
            preview_id: input.preview_id,
            changes,
            generation,
            backup,
        };
        tx.execute(
            "INSERT INTO discovery_receipts VALUES(?1,?2,?3)",
            params![
                input.preview_id.to_string(),
                owner.to_string(),
                serde_json::to_string(&result)?,
            ],
        )?;
        audit(
            &tx,
            &owner.to_string(),
            "devices.discover",
            &input.preview_id.to_string(),
            "OK",
        )?;
        tx.commit()?;
        self.discovery_sessions.remove(&input.preview_id);
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn sessions_are_bounded_expire_and_discard_without_changing_registry() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::open(&dir.path().join("db.sqlite")).unwrap();
        let input = PrepareDiscovery {
            format: DiscoverySource::Manual,
            text: "fixture".into(),
        };
        let mut last = None;
        for _ in 0..SESSION_CAPACITY {
            let owner = Uuid::new_v4();
            let p = store.prepare_discovery_session(owner, &input).unwrap();
            last = Some((owner, p.id));
        }
        assert!(matches!(
            store.prepare_discovery_session(Uuid::new_v4(), &input),
            Err(Error::DiscoveryBusy)
        ));
        let (owner, id) = last.unwrap();
        store.discovery_sessions.get_mut(&id).unwrap().expires_at =
            Utc::now() - Duration::seconds(1);
        assert!(matches!(
            store.apply_discovery_session(
                owner,
                &ApplyDiscovery {
                    preview_id: id,
                    confirmed: true
                }
            ),
            Err(Error::PreviewUnavailable)
        ));
        let p = store.prepare_discovery_session(owner, &input).unwrap();
        store.discard_discovery_session(owner);
        assert!(matches!(
            store.apply_discovery_session(
                owner,
                &ApplyDiscovery {
                    preview_id: p.id,
                    confirmed: true
                }
            ),
            Err(Error::PreviewUnavailable)
        ));
        assert_eq!(store.snapshot().unwrap().generation, 0);
        assert!(store.snapshot().unwrap().devices.is_empty());
    }
}
