use crate::{server::audit, store::*, Error, Result, Store};
use chrono::Utc;
use orkestr_domain::{ApplicationId, LibraryReference, NumericVersion};
use orkestr_protocol::*;
use rusqlite::{params, OptionalExtension, TransactionBehavior};
use uuid::Uuid;

impl Store {
    pub fn library_applications(&self) -> Result<Vec<LibraryApplication>> {
        if self
            .db
            .query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))?
            < 9
        {
            return Ok(vec![]);
        }
        json_rows(
            &self.db,
            "SELECT data FROM library_applications ORDER BY id LIMIT 1024",
        )
    }
    pub fn library_application(&self, id: ApplicationId) -> Result<Option<LibraryApplication>> {
        let data: Option<String> = self
            .db
            .query_row(
                "SELECT data FROM library_applications WHERE id=?1",
                [id.to_string()],
                |r| r.get(0),
            )
            .optional()?;
        data.map(|s| serde_json::from_str(&s).map_err(Error::from))
            .transpose()
    }
    pub fn save_application(
        &mut self,
        actor: Uuid,
        input: &SaveApplication,
    ) -> Result<LibraryApplication> {
        let mut app = input
            .application
            .normalized()
            .map_err(|_| Error::Invalid("invalid library application"))?;
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let prior: Option<u64> = tx
            .query_row(
                "SELECT revision FROM library_applications WHERE id=?1",
                [app.id.to_string()],
                |r| r.get(0),
            )
            .optional()?;
        if prior != input.expected {
            return Err(Error::Conflict);
        }
        if prior.is_none() {
            let count: i64 =
                tx.query_row("SELECT count(*) FROM library_applications", [], |r| {
                    r.get(0)
                })?;
            if count >= 1024 {
                return Err(Error::Invalid("library capacity"));
            }
        }
        app.revision = prior.unwrap_or(0) + 1;
        let data = serde_json::to_string(&app)?;
        tx.execute("INSERT INTO library_applications VALUES(?1,?2,?3) ON CONFLICT(id) DO UPDATE SET revision=excluded.revision,data=excluded.data",params![app.id.to_string(),app.revision,data])?;
        tx.execute(
            "INSERT INTO library_application_history VALUES(?1,?2,?3)",
            params![app.id.to_string(), app.revision, data],
        )?;
        audit(
            &tx,
            &actor.to_string(),
            "library.configure",
            &app.id.to_string(),
            "SOURCE_REVISION_SAVED",
        )?;
        bump(&tx)?;
        tx.commit()?;
        Ok(app)
    }
    pub fn library_releases(&self) -> Result<Vec<ReleaseCandidate>> {
        json_rows(
            &self.db,
            "SELECT data FROM library_releases ORDER BY rowid DESC LIMIT 1024",
        )
    }
    pub fn library_release(&self, id: Uuid) -> Result<Option<ReleaseCandidate>> {
        let data: Option<String> = self
            .db
            .query_row(
                "SELECT data FROM library_releases WHERE id=?1",
                [id.to_string()],
                |r| r.get(0),
            )
            .optional()?;
        data.map(|s| serde_json::from_str(&s).map_err(Error::from))
            .transpose()
    }
    fn scoped_releases(
        &self,
        id: ApplicationId,
        revision: u64,
        approved_only: bool,
    ) -> Result<Vec<ReleaseCandidate>> {
        let mut query=self.db.prepare("SELECT data FROM library_releases WHERE application_id=?1 AND application_revision=?2 AND state<>'Rejected' AND (?3=0 OR state='Approved') ORDER BY rowid")?;
        let rows = query.query_map(params![id.to_string(), revision, approved_only], |r| {
            r.get::<_, String>(0)
        })?;
        rows.map(|row| Ok(serde_json::from_str(&row?)?)).collect()
    }
    pub fn add_release(
        &mut self,
        actor: Uuid,
        app: &LibraryApplication,
        preview: SourcePreview,
    ) -> Result<ReleaseCandidate> {
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let revision: u64 = tx.query_row(
            "SELECT revision FROM library_applications WHERE id=?1",
            [app.id.to_string()],
            |r| r.get(0),
        )?;
        if revision != app.revision {
            return Err(Error::Conflict);
        }
        let release = ReleaseCandidate {
            artifact_metadata: None,
            id: Uuid::new_v4(),
            application_id: app.id,
            application_revision: app.revision,
            channel: app.channel.clone(),
            architecture: app.architecture.clone(),
            version_raw: preview.version_raw,
            version: preview.version,
            url: preview.url,
            evidence: preview.evidence,
            state: ReleaseState::Candidate,
            sha256: None,
            bytes: None,
            approved_by: None,
            approved_at: None,
        };
        tx.execute(
            "INSERT INTO library_releases VALUES(?1,?2,?3,?4,?5)",
            params![
                release.id.to_string(),
                app.id.to_string(),
                app.revision,
                "Candidate",
                serde_json::to_string(&release)?
            ],
        )?;
        audit(
            &tx,
            &actor.to_string(),
            "library.check",
            &release.id.to_string(),
            "RELEASE_CANDIDATE",
        )?;
        bump(&tx)?;
        tx.commit()?;
        Ok(release)
    }
    pub fn update_release(&mut self, actor: Uuid, release: &ReleaseCandidate) -> Result<()> {
        let prior = self.library_release(release.id)?.ok_or(Error::Conflict)?;
        if prior.application_id != release.application_id
            || prior.application_revision != release.application_revision
            || prior.version != release.version
            || prior.version_raw != release.version_raw
            || prior.url != release.url
            || (prior.sha256.is_some() && prior.sha256 != release.sha256)
            || !matches!(
                (prior.state, release.state),
                (ReleaseState::Candidate, ReleaseState::Quarantined)
                    | (ReleaseState::Quarantined, ReleaseState::Quarantined)
                    | (ReleaseState::Quarantined, ReleaseState::Verified)
                    | (ReleaseState::Verified, ReleaseState::Approved)
                    | (_, ReleaseState::Rejected)
            )
        {
            return Err(Error::Conflict);
        }
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let revision: u64 = tx.query_row(
            "SELECT revision FROM library_applications WHERE id=?1",
            [release.application_id.to_string()],
            |r| r.get(0),
        )?;
        if revision != release.application_revision {
            return Err(Error::Conflict);
        }
        if tx.execute(
            "UPDATE library_releases SET state=?2,data=?3 WHERE id=?1",
            params![
                release.id.to_string(),
                format!("{:?}", release.state),
                serde_json::to_string(release)?
            ],
        )? != 1
        {
            return Err(Error::Conflict);
        }
        audit(
            &tx,
            &actor.to_string(),
            "library.release",
            &release.id.to_string(),
            &format!("RELEASE_{:?}", release.state),
        )?;
        bump(&tx)?;
        tx.commit()?;
        Ok(())
    }
    pub fn approve_release(
        &mut self,
        actor: Uuid,
        input: &ChangeRelease,
        approve: bool,
    ) -> Result<ReleaseCandidate> {
        let mut release = self
            .library_release(input.id)?
            .ok_or(Error::Invalid("release unavailable"))?;
        if release.application_revision != input.application_revision
            || (approve && release.state != ReleaseState::Verified)
            || (!approve && release.state == ReleaseState::Rejected)
        {
            return Err(Error::Conflict);
        }
        if approve {
            // An older version never silently replaces the approved reference.
            for old in
                self.scoped_releases(release.application_id, release.application_revision, true)?
            {
                if NumericVersion::compare(&release.version, &old.version)
                    == Some(std::cmp::Ordering::Less)
                {
                    return Err(Error::Invalid("release downgrade"));
                }
            }
            release.state = ReleaseState::Approved;
            release.approved_by = Some(actor);
            release.approved_at = Some(Utc::now());
        } else {
            release.state = ReleaseState::Rejected;
        }
        self.update_release(actor, &release)?;
        Ok(release)
    }
    pub fn library_references(&self) -> Result<Vec<LibraryReference>> {
        let applications = self.library_applications()?;
        if applications.is_empty() {
            return Ok(vec![]);
        }
        let mut references = vec![];
        for app in applications {
            // Evaluation cannot inherit truncation of the bounded UI history list.
            let releases = self.scoped_releases(app.id, app.revision, false)?;
            for approved in [false, true] {
                let latest = releases
                    .iter()
                    .filter(|r| {
                        r.application_id == app.id
                            && r.application_revision == app.revision
                            && r.state != ReleaseState::Rejected
                            && (!approved || r.state == ReleaseState::Approved)
                    })
                    .max_by(|a, b| {
                        NumericVersion::compare(&a.version, &b.version)
                            .unwrap_or(std::cmp::Ordering::Equal)
                    });
                if let Some(latest) = latest {
                    references.push(LibraryReference {
                        installed_version_components: app.software_match.version_components,
                        application_id: app.id,
                        channel: app.channel.clone(),
                        architecture: app.architecture.clone(),
                        approved,
                        version: latest.version.clone(),
                    });
                }
            }
        }
        Ok(references)
    }
    pub fn library_jobs(&self) -> Result<Vec<LibraryJob>> {
        json_rows(
            &self.db,
            "SELECT data FROM library_jobs ORDER BY (state IN ('Queued','Running')) DESC,rowid DESC LIMIT 256",
        )
    }
    pub fn library_job(&self, id: Uuid) -> Result<Option<LibraryJob>> {
        let data: Option<String> = self
            .db
            .query_row(
                "SELECT data FROM library_jobs WHERE id=?1",
                [id.to_string()],
                |r| r.get(0),
            )
            .optional()?;
        data.map(|s| serde_json::from_str(&s).map_err(Error::from))
            .transpose()
    }
    pub fn queue_library_job(
        &mut self,
        actor: Uuid,
        key: &str,
        task: LibraryTask,
    ) -> Result<LibraryJob> {
        let prior: Option<String> = self
            .db
            .query_row(
                "SELECT data FROM library_jobs WHERE actor_id=?1 AND idempotency=?2",
                params![actor.to_string(), key],
                |r| r.get(0),
            )
            .optional()?;
        if let Some(data) = prior {
            let job: LibraryJob = serde_json::from_str(&data)?;
            if job.task != task {
                return Err(Error::Conflict);
            }
            return Ok(job);
        }
        let job = LibraryJob {
            attempts: 0,
            retry_at: None,
            id: Uuid::new_v4(),
            actor_id: actor,
            task,
            state: LibraryJobState::Queued,
            code: "QUEUED".into(),
            created_at: Utc::now(),
            release_id: None,
        };
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let active: u64 = tx.query_row(
            "SELECT count(*) FROM library_jobs WHERE state IN ('Queued','Running')",
            [],
            |r| r.get(0),
        )?;
        if active >= 128 {
            return Err(Error::Invalid("active library job capacity"));
        }
        tx.execute(
            "INSERT INTO library_jobs VALUES(?1,?2,?3,?4,?5)",
            params![
                job.id.to_string(),
                actor.to_string(),
                key,
                "Queued",
                serde_json::to_string(&job)?
            ],
        )?;
        audit(
            &tx,
            &actor.to_string(),
            "library.queue",
            &job.id.to_string(),
            "LIBRARY_JOB_QUEUED",
        )?;
        tx.commit()?;
        Ok(job)
    }
    pub fn set_library_job(&mut self, job: &LibraryJob) -> Result<()> {
        self.db.execute(
            "UPDATE library_jobs SET state=?2,data=?3 WHERE id=?1",
            params![
                job.id.to_string(),
                format!("{:?}", job.state),
                serde_json::to_string(job)?
            ],
        )?;
        Ok(())
    }
    pub fn recover_library_jobs(&mut self) -> Result<()> {
        let active: Vec<LibraryJob> = json_rows(
            &self.db,
            "SELECT data FROM library_jobs WHERE state='Running'",
        )?;
        for mut job in active {
            job.state = LibraryJobState::Interrupted;
            job.code = "RESTARTED_NOT_RETRIED".into();
            self.set_library_job(&job)?;
        }
        Ok(())
    }
}
