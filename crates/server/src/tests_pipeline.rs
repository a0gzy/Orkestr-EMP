//! Durable decision tests; no network access, credentials, installs or reboots.
use crate::{api::Api, automation, hash_token, new_token, scheduling};
use chrono::{Duration, Utc};
use orkestr_domain::*;
use orkestr_protocol::*;
use orkestr_storage::Store;
use std::sync::{Arc, Mutex};
use tokio::sync::{mpsc, Semaphore};
use uuid::Uuid;

struct Harness {
    _dir: tempfile::TempDir,
    api: Api,
    _receiver: mpsc::Receiver<Uuid>,
    actor: Actor,
    app: LibraryApplication,
    policy: Policy,
    baseline: ManagedInstaller,
    release: ReleaseCandidate,
}
fn release(
    store: &mut Store,
    actor: Uuid,
    app: &LibraryApplication,
    version: &str,
) -> ReleaseCandidate {
    let mut source = app.clone();
    source.source = VersionSource::Manual {
        version: version.into(),
    };
    let preview = orkestr_library::preview_source(&source, &[], None, vec![], None).unwrap();
    let mut release = store.add_release(actor, &source, preview).unwrap();
    release.state = ReleaseState::Quarantined;
    release.sha256 = Some("a".repeat(64));
    release.bytes = Some(100);
    store.update_release(actor, &release).unwrap();
    release.state = ReleaseState::Verified;
    release.artifact_metadata = Some(ArtifactMetadata {
        name: app.software_match.name.clone(),
        publisher: app.software_match.publisher.clone(),
        version: version.into(),
        product_code: Some(Uuid::new_v4()),
        architecture: Some(app.architecture.clone()),
    });
    store.update_release(actor, &release).unwrap();
    release
}
impl Harness {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::open(&dir.path().join("pipeline.sqlite")).unwrap();
        let token = new_token("Pipeline owner".into(), Role::Admin, 1).unwrap();
        store
            .bootstrap_server(
                "synthetic-owner",
                b"synthetic-blob",
                &token.actor,
                &hash_token(&token.token),
            )
            .unwrap();
        let actor = token.actor;
        let mut devices = std::collections::BTreeSet::new();
        for host in ["one.invalid", "two.invalid"] {
            let preview = store
                .preview_discovery(&orkestr_inventory::DiscoveryBatch::manual(host).unwrap())
                .unwrap();
            devices.insert(preview.writes()[0].id);
            store.apply_discovery(&preview).unwrap();
            let credential = CredentialProfile {
                id: Uuid::new_v4(),
                revision: 0,
                label: host.into(),
                username: "FIXTURE\\user".into(),
                kind: ProfileKind::Execution,
                target: host.into(),
                owner_sid: "synthetic-owner".into(),
                updated_at: Utc::now(),
            };
            let credential = store
                .save_credential(&actor, &credential, None, b"synthetic-ciphertext")
                .unwrap();
            store
                .save_connection(
                    &actor,
                    &ConnectionProfile {
                        id: Uuid::new_v4(),
                        revision: 0,
                        label: host.into(),
                        host: host.into(),
                        kind: ProfileKind::Execution,
                        port: 5986,
                        credential_id: credential.id,
                        winrm_authentication: Some(WinRmAuthentication::Kerberos),
                    },
                    None,
                )
                .unwrap();
        }
        let collection = Collection {
            id: CollectionId::new(),
            parent_id: None,
            name: "Pinned targets".into(),
            revision: 0,
            kind: CollectionKind::Static(devices),
        };
        store.save_collection(&collection, None).unwrap();
        let app:LibraryApplication=serde_json::from_value(serde_json::json!({"id":ApplicationId::new(),"revision":0,"name":"Fixture","channel":"stable","architecture":"x64","source":{"kind":"Manual","version":"1"},"download":{"kind":"Manual","url":"https://repo.example.test/fixture.exe"},"network":{"allowed_hosts":["repo.example.test"],"max_bytes":1048576},"verification":{"kind":"Authenticode","publisher":"CN=Trusted fixture"},"software_match":{"name":"Fixture","publisher":"Fixture publisher","product_code":null}})).unwrap();
        let app = store
            .save_application(
                actor.id,
                &SaveApplication {
                    application: app,
                    expected: None,
                },
            )
            .unwrap();
        let candidate = release(&mut store, actor.id, &app, "1");
        let release = store
            .approve_release(
                actor.id,
                &ChangeRelease {
                    id: candidate.id,
                    application_revision: app.revision,
                },
                true,
            )
            .unwrap();
        let baseline = ManagedInstaller {
            name: app.name.clone(),
            version: "1".into(),
            sha256: release.sha256.clone().unwrap(),
            product_code: Uuid::new_v4(),
            execution: InstallerExecution::Exe {
                arguments: "/explicit-silent".into(),
            },
            registry_key: None,
            allow_upgrade: true,
            version_components: None,
        };
        let package=store.register_installer_origin(actor.id,&baseline,serde_json::json!({"release_id":release.id,"application_id":app.id,"application_revision":app.revision})).unwrap();
        let policy = Policy {
            id: Uuid::new_v4(),
            revision: 0,
            name: "Independent flags".into(),
            application_id: app.id,
            collection_id: collection.id,
            package_revision_id: package.id,
            automation: Automation {
                auto_check: false,
                auto_download: false,
                auto_deploy: false,
                unattended: false,
            },
            interval_seconds: 60,
            jitter_seconds: 0,
            window: None,
            pilot_count: 1,
            soak_seconds: 60,
            max_failure_percent: 0,
            max_attempts: 3,
        };
        let policy = store
            .save_policy(
                actor.id,
                &SavePolicy {
                    policy,
                    expected: None,
                },
            )
            .unwrap();
        let (jobs, receiver) = mpsc::channel(8);
        let api = Api {
            store: Arc::new(Mutex::new(store)),
            jobs,
            sid: "synthetic-owner".into(),
            requests: Arc::new(Semaphore::new(16)),
            rate: Arc::new(Mutex::new((std::time::Instant::now(), 0))),
            instance_guard: None,
            artifact_directory: Some(dir.path().join("artifacts")),
            stopping: Arc::default(),
        };
        Self {
            _dir: dir,
            api,
            _receiver: receiver,
            actor,
            app,
            policy,
            baseline,
            release,
        }
    }
    fn save(&mut self) {
        let mut store = self.api.store.lock().unwrap();
        self.policy = store
            .save_policy(
                self.actor.id,
                &SavePolicy {
                    policy: self.policy.clone(),
                    expected: Some(self.policy.revision),
                },
            )
            .unwrap();
    }
    fn plan(&self, key: &str) -> Rollout {
        let mut store = self.api.store.lock().unwrap();
        let generation = store.snapshot().unwrap().generation;
        scheduling::plan(
            &mut store,
            self.actor.id,
            key,
            &PlanRollout {
                package_revision_id: None,
                policy_id: self.policy.id,
                policy_revision: self.policy.revision,
                generation,
            },
        )
        .unwrap()
    }
    fn start(&self) -> Rollout {
        let mut run = self.plan("wave");
        run.state = RolloutState::Pilot;
        self.api
            .store
            .lock()
            .unwrap()
            .update_rollout(self.actor.id, &mut run)
            .unwrap();
        run
    }
    fn current(&self, id: Uuid) -> Rollout {
        self.api.store.lock().unwrap().rollout(id).unwrap().unwrap()
    }
}

#[test]
fn signed_update_requires_current_trust_identity_version_and_unchanged_recipe() {
    let h = Harness::new();
    let mut store = h.api.store.lock().unwrap();
    let next = release(&mut store, h.actor.id, &h.app, "2");
    assert!(automation::signed_candidate(&h.app, &h.baseline, &h.release, &next).is_some());
    for changed in 0..7 {
        let mut candidate = next.clone();
        match changed {
            0 => candidate.application_revision += 1,
            1 => candidate.artifact_metadata.as_mut().unwrap().publisher = "Other publisher".into(),
            2 => candidate.artifact_metadata.as_mut().unwrap().name = "Other app".into(),
            3 => candidate.artifact_metadata.as_mut().unwrap().version = "3".into(),
            4 => candidate.version = "1".into(),
            5 => candidate.state = ReleaseState::Rejected,
            _ => candidate.artifact_metadata.as_mut().unwrap().architecture = Some("x86".into()),
        }
        assert!(
            automation::signed_candidate(&h.app, &h.baseline, &h.release, &candidate).is_none(),
            "case {changed}"
        );
    }
    let mut hash_app = h.app.clone();
    hash_app.verification = VerificationPolicy::Hash {
        expected_sha256: "a".repeat(64),
    };
    assert!(automation::signed_candidate(&hash_app, &h.baseline, &h.release, &next).is_none());
    let mut changed = h.baseline.clone();
    changed.execution = InstallerExecution::Exe {
        arguments: "/different".into(),
    };
    assert!(!automation::compatible(&h.baseline, &changed));
    // A changed MSI ProductCode also changes detection identity. A new explicit
    // recipe/policy is required; a different GUID must not disguise a downgrade.
    let mut msi = h.baseline.clone();
    msi.execution = InstallerExecution::Msi {};
    let mut msi_candidate = next.clone();
    msi_candidate
        .artifact_metadata
        .as_mut()
        .unwrap()
        .product_code = Some(Uuid::new_v4());
    assert!(automation::signed_candidate(&h.app, &msi, &h.release, &msi_candidate).is_none());
    msi_candidate
        .artifact_metadata
        .as_mut()
        .unwrap()
        .product_code = Some(msi.product_code);
    assert!(automation::signed_candidate(&h.app, &msi, &h.release, &msi_candidate).is_some());
}
#[test]
fn verified_download_is_not_approval_but_explicit_unattended_can_promote_once() {
    let h = Harness::new();
    let mut store = h.api.store.lock().unwrap();
    let candidate = release(&mut store, h.actor.id, &h.app, "2");
    let mut policy = h.policy.clone();
    policy.automation.auto_deploy = true;
    assert_eq!(
        automation::effective_package(&mut store, h.actor.id, &policy).unwrap(),
        policy.package_revision_id
    );
    assert_eq!(
        store.library_release(candidate.id).unwrap().unwrap().state,
        ReleaseState::Verified
    );
    policy.automation.unattended = true;
    let selected = automation::effective_package(&mut store, h.actor.id, &policy).unwrap();
    assert_ne!(selected, policy.package_revision_id);
    assert_eq!(
        store.library_release(candidate.id).unwrap().unwrap().state,
        ReleaseState::Approved
    );
    let count = store.packages().unwrap().len();
    assert_eq!(
        automation::effective_package(&mut store, h.actor.id, &policy).unwrap(),
        selected
    );
    assert_eq!(store.packages().unwrap().len(), count);
    assert!(automation::override_allowed(&store, &policy, selected).unwrap());
}
#[test]
fn flags_queue_only_the_selected_phase_and_default_deploy_waits_for_approval() {
    let mut h = Harness::new();
    scheduling::tick(&h.api);
    assert!(h
        .api
        .store
        .lock()
        .unwrap()
        .library_jobs()
        .unwrap()
        .is_empty());
    h.policy.automation.auto_check = true;
    h.save();
    h.api
        .store
        .lock()
        .unwrap()
        .set_policy_check_due(h.policy.id, Utc::now() - Duration::seconds(1))
        .unwrap();
    scheduling::tick(&h.api);
    assert_eq!(h.api.store.lock().unwrap().library_jobs().unwrap().len(), 1);
    assert!(h.api.store.lock().unwrap().rollouts().unwrap().is_empty());
    h.policy.automation.auto_check = false;
    h.policy.automation.auto_download = true;
    h.save();
    {
        let mut store = h.api.store.lock().unwrap();
        let preview = orkestr_library::preview_source(&h.app, &[], None, vec![], None).unwrap();
        store.add_release(h.actor.id, &h.app, preview).unwrap();
    }
    scheduling::tick(&h.api);
    assert_eq!(h.api.store.lock().unwrap().library_jobs().unwrap().len(), 2);
    assert!(h.api.store.lock().unwrap().rollouts().unwrap().is_empty());
    h.policy.automation.auto_download = false;
    h.policy.automation.auto_deploy = true;
    h.save();
    scheduling::tick(&h.api);
    let run = h.api.store.lock().unwrap().rollouts().unwrap()[0].clone();
    assert_eq!(run.state, RolloutState::WaitingApproval);
    assert!(run.targets.iter().all(|t| t.deployment_id.is_none()));
    assert!(h
        .api
        .store
        .lock()
        .unwrap()
        .deployments()
        .unwrap()
        .is_empty());
}
#[test]
fn rollout_snapshot_replays_after_generation_change_and_source_failure_blocks_admission() {
    let h = Harness::new();
    let run = h.plan("same");
    let mut store = h.api.store.lock().unwrap();
    let extra = store
        .preview_discovery(&orkestr_inventory::DiscoveryBatch::manual("later.invalid").unwrap())
        .unwrap();
    store.apply_discovery(&extra).unwrap();
    let replay = scheduling::plan(
        &mut store,
        h.actor.id,
        "same",
        &PlanRollout {
            package_revision_id: None,
            policy_id: h.policy.id,
            policy_revision: h.policy.revision,
            generation: 0,
        },
    )
    .unwrap();
    assert_eq!(replay.id, run.id);
    assert_eq!(replay.targets.len(), 2);
    let mut job = store
        .queue_library_job(
            h.actor.id,
            "broken-selector",
            LibraryTask::Check {
                application_id: h.app.id,
                revision: h.app.revision,
            },
        )
        .unwrap();
    job.state = LibraryJobState::Failed;
    job.code = "AMBIGUOUS_VERSION".into();
    store.set_library_job(&job).unwrap();
    assert!(!store
        .package_source_current(h.policy.package_revision_id)
        .unwrap());
    drop(store);
    let mut run = h.current(run.id);
    run.state = RolloutState::Pilot;
    h.api
        .store
        .lock()
        .unwrap()
        .update_rollout(h.actor.id, &mut run)
        .unwrap();
    scheduling::tick(&h.api);
    assert_eq!(h.current(run.id).state, RolloutState::Paused);
    assert!(h
        .api
        .store
        .lock()
        .unwrap()
        .deployments()
        .unwrap()
        .is_empty());
}
#[test]
fn pilot_soak_precedes_production_and_cancel_blocks_old_children() {
    let h = Harness::new();
    let run = h.start();
    scheduling::tick(&h.api);
    let current = h.current(run.id);
    assert!(current.targets[0].deployment_id.is_some());
    assert!(current.targets[1].deployment_id.is_none());
    let mut store = h.api.store.lock().unwrap();
    let mut child = store
        .deployment(current.targets[0].deployment_id.unwrap())
        .unwrap()
        .unwrap();
    child.state = DeploymentState::Succeeded;
    store.update_deployment(&mut child, h.actor.id).unwrap();
    drop(store);
    scheduling::tick(&h.api);
    let mut current = h.current(run.id);
    assert_eq!(current.state, RolloutState::Soaking);
    scheduling::tick(&h.api);
    assert!(h.current(run.id).targets[1].deployment_id.is_none());
    current.soak_until = Some(Utc::now() - Duration::seconds(1));
    h.api
        .store
        .lock()
        .unwrap()
        .update_rollout(h.actor.id, &mut current)
        .unwrap();
    scheduling::tick(&h.api);
    let mut current = h.current(run.id);
    let other = current.targets[1].deployment_id.unwrap();
    assert_eq!(current.state, RolloutState::Production);
    current.state = RolloutState::Cancelled;
    h.api
        .store
        .lock()
        .unwrap()
        .update_rollout(h.actor.id, &mut current)
        .unwrap();
    scheduling::tick(&h.api);
    let store = h.api.store.lock().unwrap();
    assert_eq!(
        store.deployment(other).unwrap().unwrap().state,
        DeploymentState::Cancelled
    );
    assert!(!store.deployment_schedule_allows(other, Utc::now()).unwrap());
}
#[test]
fn unknown_outcome_and_error_threshold_pause_without_dispatching_production() {
    for state in [
        DeploymentState::NeedsReconciliation,
        DeploymentState::Failed,
    ] {
        let h = Harness::new();
        let run = h.start();
        scheduling::tick(&h.api);
        let current = h.current(run.id);
        let mut store = h.api.store.lock().unwrap();
        let mut child = store
            .deployment(current.targets[0].deployment_id.unwrap())
            .unwrap()
            .unwrap();
        child.state = state;
        child.code = "TEST_FAILURE".into();
        store.update_deployment(&mut child, h.actor.id).unwrap();
        drop(store);
        scheduling::tick(&h.api);
        let current = h.current(run.id);
        assert_eq!(current.state, RolloutState::Paused);
        assert!(current.targets[1].deployment_id.is_none());
    }
}
#[test]
fn changed_policy_or_revoked_owner_pauses_pending_wave() {
    for revoke in [false, true] {
        let mut h = Harness::new();
        let run = h.start();
        if revoke {
            let other = new_token("Other admin".into(), Role::Admin, 1).unwrap();
            let mut store = h.api.store.lock().unwrap();
            store
                .issue_token(&h.actor, &other.actor, &hash_token(&other.token))
                .unwrap();
            store.revoke_token(&other.actor, h.actor.id).unwrap();
        } else {
            h.policy.name = "Revised".into();
            h.save();
        }
        scheduling::tick(&h.api);
        assert_eq!(h.current(run.id).state, RolloutState::Paused);
        assert!(h
            .api
            .store
            .lock()
            .unwrap()
            .deployments()
            .unwrap()
            .is_empty());
    }
}

#[test]
fn safe_retry_deadline_survives_ticks_and_dispatch_waits_until_due() {
    let mut h = Harness::new();
    h.policy.automation.unattended = true;
    h.save();
    let run = h.start();
    scheduling::tick(&h.api);
    let current = h.current(run.id);
    let id = current.targets[0].deployment_id.unwrap();
    {
        let mut store = h.api.store.lock().unwrap();
        let mut child = store.deployment(id).unwrap().unwrap();
        child.state = DeploymentState::Failed;
        child.code = "FAILED_BEFORE_INSTALL".into();
        child.attempts.push(DeploymentAttempt {
            transfer_bytes: None,
            transfer_ms: None,
            approved_by: h.actor.id,
            started_at: Utc::now(),
            finished_at: Some(Utc::now()),
            code: child.code.clone(),
            exit_code: None,
            detection: Detection::Unknown,
            safe_to_retry: true,
        });
        store.update_deployment(&mut child, h.actor.id).unwrap();
    }
    scheduling::tick(&h.api);
    let deadline = h.current(run.id).targets[0].retry_at.unwrap();
    assert!(deadline > Utc::now());
    scheduling::tick(&h.api);
    assert_eq!(h.current(run.id).targets[0].retry_at, Some(deadline));
    assert_eq!(
        h.api
            .store
            .lock()
            .unwrap()
            .deployment(id)
            .unwrap()
            .unwrap()
            .state,
        DeploymentState::Failed
    );
    let mut current = h.current(run.id);
    current.targets[0].retry_at = Some(Utc::now() - Duration::seconds(1));
    h.api
        .store
        .lock()
        .unwrap()
        .update_rollout(h.actor.id, &mut current)
        .unwrap();
    scheduling::tick(&h.api);
    assert_eq!(
        h.api
            .store
            .lock()
            .unwrap()
            .deployment(id)
            .unwrap()
            .unwrap()
            .state,
        DeploymentState::Queued
    );
    assert!(h.current(run.id).targets[0].retry_at.is_none());
}

#[test]
fn workflow_scan_failure_is_saved_without_replacing_the_previous_head() {
    use orkestr_inventory::{InventoryReport, ProviderReport};
    let h = Harness::new();
    let device = h.api.store.lock().unwrap().snapshot().unwrap().devices[0]
        .device
        .id;
    let at = Utc::now() - Duration::seconds(10);
    let previous = InventoryReport {
        id: Uuid::new_v4(),
        device_id: device,
        started_at: at,
        finished_at: at + Duration::seconds(1),
        providers: vec![ProviderReport {
            provider: InventoryProvider::MemoryModules,
            state: ProviderState::Complete,
            failure: None,
            rows: vec![serde_json::json!({"Capacity":8*GIB})],
        }],
    }
    .normalize()
    .unwrap();
    assert_eq!(
        crate::workflow::record_scan(&h.api, &previous, None).state,
        "Succeeded"
    );
    let failed = InventoryReport {
        id: Uuid::new_v4(),
        device_id: device,
        started_at: Utc::now(),
        finished_at: Utc::now(),
        providers: vec![ProviderReport {
            provider: InventoryProvider::ComputerSystem,
            state: ProviderState::Failed,
            failure: Some(ProviderFailure::Timeout),
            rows: vec![],
        }],
    }
    .normalize()
    .unwrap();
    let result = crate::workflow::record_scan(&h.api, &failed, Some("CONNECTION_TEST_TIMEOUT"));
    assert_eq!(result.state, "Failed");
    assert!(result.evidence.unwrap().contains("CONNECTION_TEST_TIMEOUT"));
    let status = h
        .api
        .store
        .lock()
        .unwrap()
        .device_inventory_status(device)
        .unwrap();
    assert_eq!(status.latest.unwrap().id, failed.report().id);
    assert_eq!(status.head.unwrap().id, previous.report().id);
}
