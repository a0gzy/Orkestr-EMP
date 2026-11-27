//! Server-only repositories. No ciphertext in metadata DTOs, audit or exports.
use crate::{store::write_collection, *};
use chrono::Utc;
use orkestr_domain::*;
use orkestr_protocol::*;
use rusqlite::{params, OptionalExtension, TransactionBehavior};
use uuid::Uuid;

pub(crate) fn audit(
    db: &rusqlite::Connection,
    actor: &str,
    verb: &str,
    target: &str,
    code: &str,
) -> Result<()> {
    db.execute(
        "INSERT INTO audit_events(at,actor,verb,target,code) VALUES(?1,?2,?3,?4,?5)",
        params![Utc::now().to_rfc3339(), actor, verb, target, code],
    )?;
    Ok(())
}
impl Store {
    pub fn server_identity(&self) -> Result<Option<(String, Vec<u8>)>> {
        Ok(self
            .db
            .query_row(
                "SELECT owner_sid,canary FROM server_identity WHERE id=1",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?)
    }
    pub fn bootstrap_server(
        &mut self,
        sid: &str,
        canary: &[u8],
        actor: &Actor,
        hash: &str,
    ) -> Result<()> {
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        if tx.query_row("SELECT count(*) FROM server_identity", [], |r| {
            r.get::<_, i64>(0)
        })? != 0
        {
            return Err(Error::Conflict);
        }
        tx.execute(
            "INSERT INTO server_identity VALUES(1,?1,?2)",
            params![sid, canary],
        )?;
        tx.execute(
            "INSERT INTO operators(id,token_hash,data) VALUES(?1,?2,?3)",
            params![actor.id.to_string(), hash, serde_json::to_string(actor)?],
        )?;
        audit(
            &tx,
            "local-bootstrap",
            "server.bootstrap",
            &actor.id.to_string(),
            "OK",
        )?;
        tx.commit()?;
        Ok(())
    }
    pub fn authenticate(&self, hash: &str) -> Result<Option<Actor>> {
        let data: Option<String> = self
            .db
            .query_row(
                "SELECT data FROM operators WHERE token_hash=?1 AND revoked=0",
                [hash],
                |r| r.get(0),
            )
            .optional()?;
        let actor: Option<Actor> = data.map(|s| serde_json::from_str(&s)).transpose()?;
        Ok(actor.filter(|a| a.expires_at > Utc::now()))
    }
    pub fn operators(&self) -> Result<Vec<Actor>> {
        json_rows(
            &self.db,
            "SELECT data FROM operators WHERE revoked=0 ORDER BY rowid",
        )
    }
    pub fn issue_token(&mut self, admin: &Actor, actor: &Actor, hash: &str) -> Result<()> {
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        tx.execute(
            "INSERT INTO operators(id,token_hash,data) VALUES(?1,?2,?3)",
            params![actor.id.to_string(), hash, serde_json::to_string(actor)?],
        )?;
        audit(
            &tx,
            &admin.id.to_string(),
            "token.issue",
            &actor.id.to_string(),
            "OK",
        )?;
        tx.commit()?;
        Ok(())
    }
    pub fn revoke_token(&mut self, admin: &Actor, id: Uuid) -> Result<()> {
        if admin.id == id {
            return Err(Error::Invalid(
                "cannot revoke the active administrator token",
            ));
        }
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        if tx.execute(
            "UPDATE operators SET revoked=1 WHERE id=?1 AND revoked=0",
            [id.to_string()],
        )? != 1
        {
            return Err(Error::Conflict);
        }
        audit(
            &tx,
            &admin.id.to_string(),
            "token.revoke",
            &id.to_string(),
            "OK",
        )?;
        tx.commit()?;
        Ok(())
    }
    pub fn credentials(&self) -> Result<Vec<CredentialProfile>> {
        json_rows(
            &self.db,
            "SELECT data FROM credential_profiles ORDER BY rowid",
        )
    }
    pub fn credential_secret(&self, id: Uuid) -> Result<(CredentialProfile, Vec<u8>)> {
        let (data, blob): (String, Vec<u8>) = self.db.query_row(
            "SELECT data,ciphertext FROM credential_profiles WHERE id=?1",
            [id.to_string()],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        Ok((serde_json::from_str(&data)?, blob))
    }
    pub fn save_credential(
        &mut self,
        actor: &Actor,
        p: &CredentialProfile,
        expected: Option<u64>,
        blob: &[u8],
    ) -> Result<CredentialProfile> {
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let old: Option<u64> = tx
            .query_row(
                "SELECT revision FROM credential_profiles WHERE id=?1",
                [p.id.to_string()],
                |r| r.get(0),
            )
            .optional()?;
        if old != expected {
            return Err(Error::Conflict);
        }
        let mut saved = p.clone();
        saved.revision = old.unwrap_or(0) + 1;
        tx.execute("INSERT INTO credential_profiles VALUES(?1,?2,?3,?4) ON CONFLICT(id) DO UPDATE SET revision=excluded.revision,data=excluded.data,ciphertext=excluded.ciphertext",params![p.id.to_string(),saved.revision,serde_json::to_string(&saved)?,blob])?;
        audit(
            &tx,
            &actor.id.to_string(),
            "credential.save",
            &p.id.to_string(),
            "OK",
        )?;
        tx.commit()?;
        Ok(saved)
    }
    pub fn connections(&self) -> Result<Vec<ConnectionProfile>> {
        json_rows(
            &self.db,
            "SELECT data FROM connection_profiles ORDER BY rowid",
        )
    }

    pub fn configure_connection(
        &mut self,
        actor: &Actor,
        configured: &ConfiguredConnection,
        blob: &[u8],
    ) -> Result<()> {
        let credential = &configured.credential;
        let connection = &configured.connection;
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let exists: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM connection_profiles WHERE id=?1)",
            [connection.id.to_string()],
            |r| r.get(0),
        )?;
        if exists {
            return Err(Error::Conflict);
        }
        tx.execute(
            "INSERT INTO credential_profiles VALUES(?1,?2,?3,?4)",
            params![
                credential.id.to_string(),
                credential.revision,
                serde_json::to_string(credential)?,
                blob
            ],
        )?;
        tx.execute(
            "INSERT INTO connection_profiles VALUES(?1,?2,?3)",
            params![
                connection.id.to_string(),
                connection.revision,
                serde_json::to_string(connection)?
            ],
        )?;
        audit(
            &tx,
            &actor.id.to_string(),
            "credential.save",
            &credential.id.to_string(),
            "OK",
        )?;
        audit(
            &tx,
            &actor.id.to_string(),
            "connection.save",
            &connection.id.to_string(),
            "OK",
        )?;
        tx.commit()?;
        Ok(())
    }
    pub fn connection(&self, id: Uuid) -> Result<Option<ConnectionProfile>> {
        let data: Option<String> = self
            .db
            .query_row(
                "SELECT data FROM connection_profiles WHERE id=?1",
                [id.to_string()],
                |r| r.get(0),
            )
            .optional()?;
        data.map(|data| serde_json::from_str(&data).map_err(Error::from))
            .transpose()
    }
    pub fn save_connection(
        &mut self,
        actor: &Actor,
        p: &ConnectionProfile,
        expected: Option<u64>,
    ) -> Result<ConnectionProfile> {
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let old: Option<(u64, String)> = tx
            .query_row(
                "SELECT revision,data FROM connection_profiles WHERE id=?1",
                [p.id.to_string()],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        if old.as_ref().map(|(revision, _)| *revision) != expected {
            return Err(Error::Conflict);
        }
        let data: String = tx.query_row(
            "SELECT data FROM credential_profiles WHERE id=?1",
            [p.credential_id.to_string()],
            |r| r.get(0),
        )?;
        let credential: CredentialProfile = serde_json::from_str(&data)?;
        if credential.kind != p.kind || credential.target != p.host {
            return Err(Error::Invalid("credential kind/target scope mismatch"));
        }
        let mut saved = p.clone();
        if let (ProfileKind::Execution, None, Some((_, data))) =
            (saved.kind, saved.winrm_authentication, old)
        {
            // A legacy client omits authentication when editing a profile it can still read.
            saved.winrm_authentication =
                serde_json::from_str::<ConnectionProfile>(&data)?.winrm_authentication;
        }
        match (saved.kind, saved.winrm_authentication) {
            (ProfileKind::AdRead, Some(_)) => {
                return Err(Error::Invalid(
                    "WinRM authentication requires an execution profile",
                ));
            }
            (_, Some(WinRmAuthentication::Kerberos))
                if !saved.host.contains('.') || saved.host.parse::<std::net::IpAddr>().is_ok() =>
            {
                return Err(Error::Invalid(
                    "Kerberos requires a fully qualified DNS name",
                ));
            }
            _ => {}
        }
        saved.revision = expected.unwrap_or(0) + 1;
        tx.execute("INSERT INTO connection_profiles VALUES(?1,?2,?3) ON CONFLICT(id) DO UPDATE SET revision=excluded.revision,data=excluded.data",params![p.id.to_string(),saved.revision,serde_json::to_string(&saved)?])?;
        audit(
            &tx,
            &actor.id.to_string(),
            "connection.save",
            &p.id.to_string(),
            "OK",
        )?;
        tx.commit()?;
        Ok(saved)
    }
    pub fn server_save_collection(
        &mut self,
        actor: &Actor,
        c: &Collection,
        expected: Option<u64>,
    ) -> Result<Collection> {
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let saved = write_collection(&tx, c, expected)?;
        crate::store::bump(&tx)?;
        audit(
            &tx,
            &actor.id.to_string(),
            "collection.save",
            &c.id.to_string(),
            "OK",
        )?;
        tx.commit()?;
        Ok(saved)
    }
    pub fn server_delete_collection(
        &mut self,
        actor: &Actor,
        id: CollectionId,
        expected: u64,
    ) -> Result<()> {
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let children: i64 = tx.query_row(
            "SELECT count(*) FROM collections WHERE parent_id=?1",
            [id.to_string()],
            |r| r.get(0),
        )?;
        if children != 0 {
            return Err(Error::Invalid("move children before deleting folder"));
        }
        if tx.execute(
            "DELETE FROM collections WHERE id=?1 AND revision=?2",
            params![id.to_string(), expected],
        )? != 1
        {
            return Err(Error::Conflict);
        }
        crate::store::bump(&tx)?;
        audit(
            &tx,
            &actor.id.to_string(),
            "collection.delete",
            &id.to_string(),
            "OK",
        )?;
        tx.commit()?;
        Ok(())
    }
    pub fn events(&self, after: u64) -> Result<Events> {
        let latest: u64 = self.db.query_row(
            "SELECT coalesce(max(cursor),0) FROM audit_events",
            [],
            |r| r.get(0),
        )?;
        let oldest = latest.saturating_sub(EVENT_LIMIT as u64);
        let resync = after < oldest || after > latest;
        let cursor = if resync { oldest } else { after };
        let mut q = self.db.prepare("SELECT cursor,at,actor,verb,target,code FROM audit_events WHERE cursor>?1 ORDER BY cursor LIMIT 128")?;
        let rows = q.query_map([cursor], |r| {
            Ok(AuditEvent {
                cursor: r.get(0)?,
                at: r
                    .get::<_, String>(1)?
                    .parse()
                    .map_err(|_| rusqlite::Error::InvalidQuery)?,
                actor: r.get(2)?,
                verb: r.get(3)?,
                target: r.get(4)?,
                code: r.get(5)?,
            })
        })?;
        let events = rows.collect::<std::result::Result<Vec<_>, _>>()?;
        let next = events.last().map_or(latest, |e| e.cursor);
        Ok(Events {
            events,
            next,
            resync,
        })
    }
    pub fn audit_denied(&self, actor: &Actor) -> Result<()> {
        audit(
            &self.db,
            &actor.id.to_string(),
            "access.denied",
            "api",
            "FORBIDDEN",
        )
    }
    pub fn existing_job(&self, actor: Uuid, key: &str) -> Result<Option<Job>> {
        let data: Option<String> = self
            .db
            .query_row(
                "SELECT data FROM server_jobs WHERE actor_id=?1 AND idempotency=?2",
                params![actor.to_string(), key],
                |r| r.get(0),
            )
            .optional()?;
        Ok(data.map(|s| serde_json::from_str(&s)).transpose()?)
    }
    pub fn job(&self, id: Uuid) -> Result<Job> {
        let data: String = self.db.query_row(
            "SELECT data FROM server_jobs WHERE id=?1",
            [id.to_string()],
            |r| r.get(0),
        )?;
        Ok(serde_json::from_str(&data)?)
    }
    pub fn create_inventory_job(
        &mut self,
        actor: &Actor,
        key: &str,
        p: &ConnectionProfile,
        task: InventoryTask,
    ) -> Result<Job> {
        let (credential, _) = self.credential_secret(p.credential_id)?;
        if credential.kind != p.kind || credential.target != p.host {
            return Err(Error::Invalid("credential kind/target scope mismatch"));
        }
        let job = Job {
            id: Uuid::new_v4(),
            actor_id: actor.id,
            connection_id: p.id,
            state: JobState::Queued,
            code: "QUEUED".into(),
            created_at: Utc::now(),
            task,
            discovery: None,
        };
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        tx.execute(
            "INSERT INTO server_jobs VALUES(?1,?2,?3,?4,?5,?6)",
            params![
                job.id.to_string(),
                actor.id.to_string(),
                key,
                serde_json::to_string(&job)?,
                p.revision,
                credential.revision
            ],
        )?;
        audit(
            &tx,
            &actor.id.to_string(),
            match job.task {
                InventoryTask::TestConnection => "connection.test",
                InventoryTask::DiscoverOu { .. } => "inventory.discover",
                InventoryTask::ScanDevice { .. } => "inventory.scan",
            },
            &job.id.to_string(),
            "QUEUED",
        )?;
        tx.commit()?;
        Ok(job)
    }
    pub fn job_revisions(&self, id: Uuid) -> Result<(u64, u64)> {
        Ok(self.db.query_row(
            "SELECT connection_revision,credential_revision FROM server_jobs WHERE id=?1",
            [id.to_string()],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?)
    }
    pub fn set_job(&mut self, job: &Job) -> Result<()> {
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        write_job_state(&tx, job)?;
        tx.commit()?;
        Ok(())
    }
    pub fn interrupt_jobs(&mut self) -> Result<()> {
        let jobs: Vec<Job> = json_rows(&self.db, "SELECT data FROM server_jobs ORDER BY rowid")?;
        for mut job in jobs {
            if matches!(job.state, JobState::Queued | JobState::Running) {
                job.state = JobState::Interrupted;
                job.code = "SERVER_RESTARTED_NO_RETRY".into();
                self.set_job(&job)?;
            }
        }
        Ok(())
    }
}

pub(crate) fn write_job_state(tx: &rusqlite::Transaction<'_>, job: &Job) -> Result<()> {
    tx.execute(
        "UPDATE server_jobs SET data=?2 WHERE id=?1",
        params![job.id.to_string(), serde_json::to_string(job)?],
    )?;
    audit(
        tx,
        &job.actor_id.to_string(),
        "job.state",
        &job.id.to_string(),
        &job.code,
    )
}
