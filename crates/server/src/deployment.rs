use crate::api::Api;
use chrono::Utc;
use orkestr_protocol::*;
use orkestr_storage::{Error, Result, Store};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File, OpenOptions},
    io::{self, Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    time::Duration,
};
use uuid::Uuid;
use zeroize::Zeroizing;

const ARTIFACT_LIMIT: u64 = 256 * 1024 * 1024;
const MSI_PREFIX: [u8; 8] = [0xd0, 0xcf, 0x11, 0xe0, 0xa1, 0xb1, 0x1a, 0xe1];
const MAX_ATTEMPTS: usize = 3;

pub(crate) fn valid_hash(hash: &str) -> bool {
    hash.len() == 64
        && hash
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

pub fn stage_msi(directory: &Path, source: &Path) -> std::result::Result<String, &'static str> {
    stage_installer(directory, source, &InstallerExecution::Msi {})
}

pub fn stage_exe(directory: &Path, source: &Path) -> std::result::Result<String, &'static str> {
    stage_installer(
        directory,
        source,
        &InstallerExecution::Exe {
            arguments: String::new(),
        },
    )
}

fn stage_installer(
    directory: &Path,
    source: &Path,
    execution: &InstallerExecution,
) -> std::result::Result<String, &'static str> {
    crate::platform::protected_directory(directory, false)
        .map_err(|_| "ARTIFACT_DIRECTORY_UNSAFE")?;
    if !source.is_absolute()
        || source.to_string_lossy().starts_with(r"\\")
        || !source
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case(execution.extension()))
    {
        return Err(if execution.is_msi() {
            "LOCAL_MSI_REQUIRED"
        } else {
            "LOCAL_EXE_REQUIRED"
        });
    }
    let mut input = open_artifact(source)?;
    let repository = directory.join("artifacts");
    fs::create_dir_all(&repository).map_err(|_| "ARTIFACT_DIRECTORY_UNAVAILABLE")?;
    let mut temp =
        tempfile::NamedTempFile::new_in(&repository).map_err(|_| "ARTIFACT_WRITE_FAILED")?;
    let hash = copy_installer(&mut input, temp.as_file_mut(), execution)?;
    temp.as_file()
        .sync_all()
        .map_err(|_| "ARTIFACT_WRITE_FAILED")?;
    let destination = repository.join(format!("{hash}.{}", execution.extension()));
    if let Err(error) = temp.persist_noclobber(&destination) {
        if error.error.kind() != io::ErrorKind::AlreadyExists {
            return Err("ARTIFACT_WRITE_FAILED");
        }
        let mut existing = open_artifact(&destination)?;
        if copy_installer(&mut existing, &mut io::sink(), execution)? != hash {
            return Err("ARTIFACT_HASH_MISMATCH");
        }
    }
    Ok(hash)
}

pub(crate) fn artifact_path(api: &Api, installer: &ManagedInstaller) -> Option<PathBuf> {
    valid_hash(&installer.sha256)
        .then(|| {
            api.artifact_directory.as_ref().map(|root| {
                root.join(format!(
                    "{}.{}",
                    installer.sha256,
                    installer.execution.extension()
                ))
            })
        })
        .flatten()
}

pub(crate) fn open_artifact(path: &Path) -> std::result::Result<File, &'static str> {
    let metadata = fs::symlink_metadata(path).map_err(|_| "ARTIFACT_UNAVAILABLE")?;
    if !metadata.is_file() || metadata.len() > ARTIFACT_LIMIT {
        return Err("ARTIFACT_UNAVAILABLE");
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if metadata.file_attributes() & 0x400 != 0 {
            return Err("ARTIFACT_UNAVAILABLE");
        }
    }
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.share_mode(1);
    }
    options.open(path).map_err(|_| "ARTIFACT_UNAVAILABLE")
}

fn copy_installer(
    input: &mut File,
    output: &mut impl Write,
    execution: &InstallerExecution,
) -> std::result::Result<String, &'static str> {
    validate_file(input, execution)?;
    input
        .seek(SeekFrom::Start(0))
        .map_err(|_| "ARTIFACT_UNAVAILABLE")?;
    let mut hash = Sha256::new();
    let mut total = 0;
    let mut block = vec![0; 128 * 1024];
    loop {
        let len = input.read(&mut block).map_err(|_| "ARTIFACT_UNAVAILABLE")?;
        if len == 0 {
            break;
        }
        total += len as u64;
        if total > ARTIFACT_LIMIT {
            return Err("ARTIFACT_TOO_LARGE");
        }
        hash.update(&block[..len]);
        output
            .write_all(&block[..len])
            .map_err(|_| "ARTIFACT_WRITE_FAILED")?;
    }
    Ok(format!("{:x}", hash.finalize()))
}

fn validate_file(
    input: &mut File,
    execution: &InstallerExecution,
) -> std::result::Result<(), &'static str> {
    input
        .seek(SeekFrom::Start(0))
        .map_err(|_| "ARTIFACT_UNAVAILABLE")?;
    if execution.is_msi() {
        let mut prefix = [0; MSI_PREFIX.len()];
        input
            .read_exact(&mut prefix)
            .map_err(|_| "INVALID_MSI_FILE")?;
        return if prefix == MSI_PREFIX {
            Ok(())
        } else {
            Err("INVALID_MSI_FILE")
        };
    }
    // Structural PE check only; trust/signature policy remains part of M4.
    let mut dos = [0; 64];
    input.read_exact(&mut dos).map_err(|_| "INVALID_EXE_FILE")?;
    let offset = u32::from_le_bytes(dos[60..64].try_into().unwrap()) as u64;
    if &dos[..2] != b"MZ" || !(64..=1024 * 1024).contains(&offset) {
        return Err("INVALID_EXE_FILE");
    }
    input
        .seek(SeekFrom::Start(offset))
        .map_err(|_| "INVALID_EXE_FILE")?;
    let mut pe = [0; 26];
    input.read_exact(&mut pe).map_err(|_| "INVALID_EXE_FILE")?;
    let flags = u16::from_le_bytes([pe[22], pe[23]]);
    let optional_size = u16::from_le_bytes([pe[20], pe[21]]);
    let magic = u16::from_le_bytes([pe[24], pe[25]]);
    if &pe[..4] != b"PE\0\0"
        || flags & 0x0002 == 0
        || flags & 0x2000 != 0
        || optional_size < 2
        || !matches!(magic, 0x10b | 0x20b)
    {
        return Err("INVALID_EXE_FILE");
    }
    Ok(())
}

pub(crate) fn target(
    store: &Store,
    plan: &PlanDeployment,
    pinned_host: Option<&str>,
    reconcile: bool,
) -> Result<(ConnectionProfile, CredentialProfile, Vec<u8>)> {
    if !reconcile && !store.package_source_current(plan.package_revision_id)? {
        return Err(Error::Conflict);
    }
    let device = store
        .device(plan.device_id)?
        .ok_or(Error::Invalid("deployment device unavailable"))?;
    let connection = store
        .connection(plan.connection_id)?
        .ok_or(Error::Invalid("deployment profile unavailable"))?;
    if (!reconcile && (device.disabled || connection.revision != plan.connection_revision))
        || connection.kind != ProfileKind::Execution
        || connection.port != 5986
        || connection.host != device.host
        || pinned_host.is_some_and(|host| host != device.host)
    {
        return Err(Error::Invalid("deployment target scope mismatch"));
    }
    let (credential, blob) = store.credential_secret(connection.credential_id)?;
    if credential.kind != ProfileKind::Execution || credential.target != connection.host {
        return Err(Error::Invalid("deployment credential scope mismatch"));
    }
    Ok((connection, credential, blob))
}

pub(crate) fn transition(
    store: &mut Store,
    record: &mut Deployment,
    actor: Uuid,
    action: &str,
) -> Result<()> {
    let (state, code) = match (action, record.state) {
        ("approve", DeploymentState::WaitingApproval) => {
            (DeploymentState::Queued, "APPROVED_QUEUED")
        }
        ("retry", DeploymentState::Failed)
            if record.attempts.len() < MAX_ATTEMPTS
                && (record.code == "RESTARTED_BEFORE_START"
                    || record.attempts.last().is_some_and(|a| a.safe_to_retry)) =>
        {
            (DeploymentState::Queued, "APPROVED_RETRY_QUEUED")
        }
        ("cancel", DeploymentState::WaitingApproval | DeploymentState::Queued) => {
            (DeploymentState::Cancelled, "CANCELLED_BEFORE_START")
        }
        ("reconcile", DeploymentState::NeedsReconciliation) => {
            (DeploymentState::ReconcileQueued, "RECONCILIATION_QUEUED")
        }
        _ => return Err(Error::Conflict),
    };
    if state == DeploymentState::Queued {
        let (_, credential, _) = target(store, &record.plan, Some(&record.host), false)?;
        if credential.revision != record.credential_revision {
            return Err(Error::Conflict);
        }
        if record.approved_by.is_none() {
            record.approved_by = Some(actor);
            record.approved_at = Some(Utc::now());
        }
    }
    if state == DeploymentState::ReconcileQueued {
        target(store, &record.plan, Some(&record.host), true)?;
    }
    record.action_by = Some(actor);
    record.state = state;
    record.code = code.into();
    store.update_deployment(record, actor)
}

#[derive(Deserialize)]
enum Outcome {
    BeforeInstall,
    AlreadyPresent,
    Finished,
    Indeterminate,
    DetectionOnly,
}

#[derive(Deserialize)]
pub(crate) struct AdapterResult {
    #[serde(default)]
    transfer_bytes: Option<u64>,
    #[serde(default)]
    transfer_ms: Option<u64>,
    outcome: Outcome,
    exit_code: Option<i32>,
    detection: Detection,
}

pub(crate) fn execute(api: &Api, id: Uuid) {
    let prepared = (|| {
        let mut store = api.store.lock().ok()?;
        let mut record = store.deployment(id).ok()??;
        let reconcile = record.state == DeploymentState::ReconcileQueued;
        if !reconcile && record.state != DeploymentState::Queued {
            return None;
        }
        let actor = record.action_by?;
        let now = Utc::now();
        if !reconcile && !store.deployment_schedule_allows(id, now).ok()? {
            return None;
        }
        let options = (|| {
            if !store
                .operators()
                .ok()?
                .iter()
                .any(|a| a.id == actor && a.role == Role::Admin && a.expires_at > now)
            {
                return None;
            }
            let (connection, credential, blob) =
                target(&store, &record.plan, Some(&record.host), reconcile).ok()?;
            if !reconcile && credential.revision != record.credential_revision {
                return None;
            }
            let secret = orkestr_credentials::unprotect(&credential.owner_sid, &blob).ok()?;
            Some((connection, credential, secret))
        })();
        if options.is_none() {
            record.state = if reconcile {
                DeploymentState::NeedsReconciliation
            } else {
                DeploymentState::Failed
            };
            record.code = "DEPLOYMENT_SCOPE_OR_AUTHORIZATION_CHANGED".into();
            let _ = store.update_deployment(&mut record, actor);
            return None;
        }
        let site = store.device(record.plan.device_id).ok()??.site;
        if !store
            .acquire_resource(
                id,
                &site,
                if reconcile {
                    Phase::ReadOnly
                } else {
                    Phase::Install
                },
            )
            .ok()?
        {
            return None;
        }
        record.state = if reconcile {
            DeploymentState::Reconciling
        } else {
            DeploymentState::Running
        };
        record.code = if reconcile {
            "RECONCILING"
        } else {
            "INSTALL_ATTEMPT_STARTED"
        }
        .into();
        if !reconcile {
            record.attempts.push(DeploymentAttempt {
                transfer_bytes: None,
                transfer_ms: None,
                approved_by: actor,
                started_at: now,
                finished_at: None,
                code: record.code.clone(),
                exit_code: None,
                detection: Detection::Unknown,
                safe_to_retry: false,
            });
        }
        if store.update_deployment(&mut record, actor).is_err() {
            let _ = store.release_resource(id);
            return None;
        }
        Some((record, actor, reconcile, options?))
    })();
    let Some((mut record, actor, reconcile, (connection, credential, secret))) = prepared else {
        return;
    };
    let result = run_adapter(api, &record, &connection, &credential, secret, reconcile);
    apply_result(&mut record, result, reconcile);
    if let Ok(mut store) = api.store.lock() {
        let _ = store.update_deployment(&mut record, actor);
        let _ = store.release_resource(id);
    }
}

pub(crate) fn run_adapter(
    api: &Api,
    record: &Deployment,
    connection: &ConnectionProfile,
    credential: &CredentialProfile,
    secret: Zeroizing<Vec<u8>>,
    reconcile: bool,
) -> AdapterResult {
    let mut path = None;
    let mut artifact = None;
    if !reconcile {
        let preflight = (|| {
            if !record.installer.valid() {
                return Err("INVALID_INSTALLER_MANIFEST");
            }
            let source = artifact_path(api, &record.installer).ok_or("ARTIFACT_UNAVAILABLE")?;
            let mut file = open_artifact(&source)?;
            if copy_installer(&mut file, &mut io::sink(), &record.installer.execution)?
                != record.installer.sha256
            {
                return Err("ARTIFACT_HASH_MISMATCH");
            }
            Ok((source, file))
        })();
        match preflight {
            Ok((source, file)) => {
                path = Some(source);
                artifact = Some(file);
            }
            Err(_) => {
                return AdapterResult {
                    transfer_bytes: None,
                    transfer_ms: None,
                    outcome: Outcome::BeforeInstall,
                    exit_code: None,
                    detection: Detection::Unknown,
                }
            }
        }
    }
    let options = serde_json::json!({ "path":path, "sha256":record.installer.sha256, "product_code":record.installer.product_code,
        "version":record.installer.version,"allow_upgrade":record.installer.allow_upgrade,"version_components":record.installer.version_components,"registry_key":record.installer.registry_key, "execution":record.installer.execution, "reconcile":reconcile,"bytes_per_second":transfer_rate(api) });
    let result = crate::probe::query(
        &format!(
            "{}\n{}",
            include_str!("transfer.ps1"),
            include_str!("deploy-installer.ps1")
        ),
        connection,
        credential,
        secret,
        &options,
        crate::probe::QueryLimits {
            deadline: Duration::from_secs(if reconcile { 45 } else { 900 }),
            max_output: 4096,
            stopping: Some(&api.stopping),
        },
    );
    drop(artifact);
    result
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or(AdapterResult {
            transfer_bytes: None,
            transfer_ms: None,
            outcome: Outcome::Indeterminate,
            exit_code: None,
            detection: Detection::Unknown,
        })
}

pub(crate) fn workflow_result(
    result: AdapterResult,
    reconcile: bool,
) -> crate::workflow::StepResult {
    let state = if reconcile {
        if matches!(result.outcome, Outcome::DetectionOnly)
            && result.detection == Detection::Present
        {
            "Succeeded"
        } else {
            "Unknown"
        }
    } else {
        match (result.outcome, result.exit_code, result.detection) {
            (Outcome::BeforeInstall, _, Detection::DifferentVersion) => "Failed",
            (Outcome::BeforeInstall, _, _) => "BeforeStep",
            (Outcome::AlreadyPresent, _, Detection::Present)
            | (Outcome::Finished, Some(0), Detection::Present) => "Succeeded",
            (Outcome::Finished, Some(3010), Detection::Present) => "RebootRequired",
            (Outcome::Finished, Some(code), Detection::Absent)
                if !matches!(code, 0 | 3010 | 1641) =>
            {
                "Failed"
            }
            _ => "Unknown",
        }
    };
    crate::workflow::step_result(state, result.exit_code)
}

pub fn stage_file(directory: &Path, source: &Path) -> std::result::Result<String, &'static str> {
    crate::platform::protected_directory(directory, false)
        .map_err(|_| "ARTIFACT_DIRECTORY_UNSAFE")?;
    if !source.is_absolute() || source.to_string_lossy().starts_with(r"\\") {
        return Err("LOCAL_FILE_REQUIRED");
    }
    let mut input = open_artifact(source)?;
    let repository = directory.join("artifacts");
    fs::create_dir_all(&repository).map_err(|_| "ARTIFACT_WRITE_FAILED")?;
    let mut temp =
        tempfile::NamedTempFile::new_in(&repository).map_err(|_| "ARTIFACT_WRITE_FAILED")?;
    let mut hash = Sha256::new();
    let mut bytes = [0; 65536];
    let mut total = 0;
    loop {
        let n = input.read(&mut bytes).map_err(|_| "ARTIFACT_UNAVAILABLE")?;
        if n == 0 {
            break;
        }
        total += n as u64;
        if total > ARTIFACT_LIMIT {
            return Err("ARTIFACT_TOO_LARGE");
        }
        hash.update(&bytes[..n]);
        temp.write_all(&bytes[..n])
            .map_err(|_| "ARTIFACT_WRITE_FAILED")?;
    }
    temp.as_file()
        .sync_all()
        .map_err(|_| "ARTIFACT_WRITE_FAILED")?;
    let hash = format!("{:x}", hash.finalize());
    let destination = repository.join(format!("{hash}.bin"));
    if let Err(error) = temp.persist_noclobber(&destination) {
        if error.error.kind() != io::ErrorKind::AlreadyExists {
            return Err("ARTIFACT_WRITE_FAILED");
        }
        let mut existing = open_artifact(&destination)?;
        let mut computed = Sha256::new();
        loop {
            let n = existing
                .read(&mut bytes)
                .map_err(|_| "ARTIFACT_UNAVAILABLE")?;
            if n == 0 {
                break;
            }
            computed.update(&bytes[..n]);
        }
        if format!("{:x}", computed.finalize()) != hash {
            return Err("ARTIFACT_HASH_MISMATCH");
        }
    }
    Ok(hash)
}

fn apply_result(record: &mut Deployment, result: AdapterResult, reconcile: bool) {
    let (state, code, retry) = if reconcile {
        record.reconciliation = Some(result.detection);
        if matches!(result.outcome, Outcome::DetectionOnly)
            && result.detection == Detection::Present
        {
            if record.attempts.last().and_then(|a| a.exit_code) == Some(3010) {
                (
                    DeploymentState::RebootRequired,
                    "RECONCILED_REBOOT_REQUIRED",
                    false,
                )
            } else {
                (
                    DeploymentState::Succeeded,
                    "RECONCILED_VERSION_PRESENT",
                    false,
                )
            }
        } else {
            (
                DeploymentState::NeedsReconciliation,
                "RECONCILIATION_UNCONFIRMED",
                false,
            )
        }
    } else {
        match (result.outcome, result.exit_code, result.detection) {
            (Outcome::BeforeInstall, _, Detection::DifferentVersion) => {
                (DeploymentState::Failed, "DIFFERENT_VERSION_PRESENT", false)
            }
            (Outcome::BeforeInstall, _, _) => {
                (DeploymentState::Failed, "FAILED_BEFORE_INSTALL", true)
            }
            (Outcome::AlreadyPresent, _, Detection::Present) => {
                (DeploymentState::Succeeded, "ALREADY_INSTALLED", false)
            }
            (Outcome::Finished, Some(0), Detection::Present) => {
                (DeploymentState::Succeeded, "INSTALL_VERIFIED", false)
            }
            (Outcome::Finished, Some(3010), Detection::Present) => (
                DeploymentState::RebootRequired,
                "INSTALL_VERIFIED_REBOOT_REQUIRED",
                false,
            ),
            (Outcome::Finished, Some(code), Detection::Absent)
                if !matches!(code, 0 | 3010 | 1641) =>
            {
                (DeploymentState::Failed, "INSTALLER_FAILED", false)
            }
            _ => (
                DeploymentState::NeedsReconciliation,
                "INSTALL_OUTCOME_UNCONFIRMED",
                false,
            ),
        }
    };
    if !reconcile {
        if let Some(attempt) = record.attempts.last_mut() {
            attempt.finished_at = Some(Utc::now().max(attempt.started_at));
            attempt.code = code.into();
            attempt.exit_code = result.exit_code;
            attempt.detection = result.detection;
            attempt.safe_to_retry = retry;
            attempt.transfer_bytes = result.transfer_bytes;
            attempt.transfer_ms = result.transfer_ms;
        }
    }
    record.state = state;
    record.code = code.into();
}

pub(crate) fn transfer_rate(api: &Api) -> u64 {
    api.store
        .lock()
        .ok()
        .and_then(|store| store.quotas().ok())
        .map(|(_, quotas)| {
            let slots = if quotas.strict_sequential {
                1
            } else {
                quotas
                    .global
                    .min(quotas.transfers)
                    .min(quotas.installs)
                    .max(1)
            };
            if quotas.endpoint_bytes_per_second == 0 {
                0
            } else {
                (quotas.endpoint_bytes_per_second / (slots as u64)).max(1)
            }
        })
        .unwrap_or(1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn running_exe() -> Deployment {
        let actor = Uuid::new_v4();
        Deployment {
            manual_resolution: None,
            id: Uuid::new_v4(),
            revision: 3,
            plan: PlanDeployment {
                device_id: orkestr_domain::DeviceId::new(),
                package_revision_id: orkestr_domain::PackageRevisionId::new(),
                connection_id: Uuid::new_v4(),
                connection_revision: 1,
            },
            installer: ManagedInstaller {
                allow_upgrade: false,
                version_components: None,
                name: "Fixture".into(),
                version: "1".into(),
                sha256: "a".repeat(64),
                product_code: Uuid::new_v4(),
                registry_key: None,
                execution: InstallerExecution::Exe {
                    arguments: "/S".into(),
                },
            },
            host: "fixture.invalid".into(),
            created_by: actor,
            approved_by: Some(actor),
            approved_at: Some(Utc::now()),
            action_by: Some(actor),
            credential_revision: 1,
            state: DeploymentState::Running,
            code: "INSTALL_ATTEMPT_STARTED".into(),
            created_at: Utc::now(),
            attempts: vec![DeploymentAttempt {
                transfer_bytes: None,
                transfer_ms: None,
                approved_by: actor,
                started_at: Utc::now(),
                finished_at: None,
                code: "INSTALL_ATTEMPT_STARTED".into(),
                exit_code: None,
                detection: Detection::Unknown,
                safe_to_retry: false,
            }],
            reconciliation: None,
        }
    }

    #[test]
    fn exe_success_needs_detection_and_uncertain_outcomes_only_allow_read_only_reconciliation() {
        for (outcome, exit_code, detection, expected, retry) in [
            (
                Outcome::Finished,
                Some(0),
                Detection::Present,
                DeploymentState::Succeeded,
                false,
            ),
            (
                Outcome::Finished,
                Some(3010),
                Detection::Present,
                DeploymentState::RebootRequired,
                false,
            ),
            (
                Outcome::Finished,
                Some(0),
                Detection::Absent,
                DeploymentState::NeedsReconciliation,
                false,
            ),
            (
                Outcome::Finished,
                Some(1641),
                Detection::Present,
                DeploymentState::NeedsReconciliation,
                false,
            ),
            (
                Outcome::Indeterminate,
                None,
                Detection::Present,
                DeploymentState::NeedsReconciliation,
                false,
            ),
            (
                Outcome::Finished,
                Some(1603),
                Detection::Absent,
                DeploymentState::Failed,
                false,
            ),
            (
                Outcome::BeforeInstall,
                None,
                Detection::Unknown,
                DeploymentState::Failed,
                true,
            ),
            (
                Outcome::BeforeInstall,
                None,
                Detection::DifferentVersion,
                DeploymentState::Failed,
                false,
            ),
        ] {
            let mut record = running_exe();
            apply_result(
                &mut record,
                AdapterResult {
                    transfer_bytes: None,
                    transfer_ms: None,
                    outcome,
                    exit_code,
                    detection,
                },
                false,
            );
            assert_eq!(record.state, expected);
            assert_eq!(record.attempts[0].safe_to_retry, retry);
            assert!(record.attempts[0].finished_at.is_some());
            if expected == DeploymentState::NeedsReconciliation {
                assert!(record.state.holds_device());
                let dir = tempfile::tempdir().unwrap();
                let mut store = Store::open(&dir.path().join("test.sqlite")).unwrap();
                let actor = record.created_by;
                assert!(matches!(
                    transition(&mut store, &mut record, actor, "retry"),
                    Err(Error::Conflict)
                ));
                apply_result(
                    &mut record,
                    AdapterResult {
                        transfer_bytes: None,
                        transfer_ms: None,
                        outcome: Outcome::DetectionOnly,
                        exit_code: None,
                        detection: Detection::Absent,
                    },
                    true,
                );
                assert_eq!(record.state, DeploymentState::NeedsReconciliation);
                apply_result(
                    &mut record,
                    AdapterResult {
                        transfer_bytes: None,
                        transfer_ms: None,
                        outcome: Outcome::DetectionOnly,
                        exit_code: None,
                        detection: Detection::Present,
                    },
                    true,
                );
                assert_eq!(record.state, DeploymentState::Succeeded);
                assert_eq!(record.attempts.len(), 1);
            }
        }
    }

    fn exe_bytes() -> Vec<u8> {
        let mut bytes = vec![0; 90];
        bytes[..2].copy_from_slice(b"MZ");
        bytes[60..64].copy_from_slice(&64u32.to_le_bytes());
        bytes[64..68].copy_from_slice(b"PE\0\0");
        bytes[84..86].copy_from_slice(&2u16.to_le_bytes());
        bytes[86..88].copy_from_slice(&2u16.to_le_bytes());
        bytes[88..90].copy_from_slice(&0x20bu16.to_le_bytes());
        bytes
    }

    #[test]
    fn pe_validation_rejects_scripts_dlls_and_truncated_headers_and_hashes_all_bytes() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("fixture.exe");
        let execution = InstallerExecution::Exe {
            arguments: "/S".into(),
        };
        let bytes = exe_bytes();
        std::fs::write(&source, &bytes).unwrap();
        let mut output = Vec::new();
        assert_eq!(
            copy_installer(&mut File::open(&source).unwrap(), &mut output, &execution).unwrap(),
            format!("{:x}", Sha256::digest(&bytes))
        );
        assert_eq!(output, bytes);
        let mut dll = bytes.clone();
        dll[86..88].copy_from_slice(&0x2002u16.to_le_bytes());
        let mut bad_offset = bytes.clone();
        bad_offset[60..64].copy_from_slice(&u32::MAX.to_le_bytes());
        for invalid in [
            b"echo not an executable".to_vec(),
            bytes[..89].to_vec(),
            dll,
            bad_offset,
        ] {
            std::fs::write(&source, invalid).unwrap();
            assert_eq!(
                copy_installer(
                    &mut File::open(&source).unwrap(),
                    &mut Vec::new(),
                    &execution
                ),
                Err("INVALID_EXE_FILE")
            );
        }
        std::fs::write(&source, &bytes).unwrap();
        assert_eq!(
            copy_installer(
                &mut File::open(&source).unwrap(),
                &mut Vec::new(),
                &InstallerExecution::Msi {}
            ),
            Err("INVALID_MSI_FILE")
        );
    }

    #[test]
    fn legacy_manifests_remain_msi_and_exe_arguments_require_an_explicit_bounded_command_line() {
        let legacy = json!({"name":"Fixture", "version":"1", "sha256":"a".repeat(64), "product_code":Uuid::new_v4()});
        let installer: ManagedInstaller = serde_json::from_value(legacy.clone()).unwrap();
        assert!(installer.execution.is_msi());
        assert_eq!(serde_json::to_value(installer).unwrap(), legacy);
        for arguments in [
            "".into(),
            "  ".into(),
            "/S\ncommand".into(),
            "x".repeat(2049),
            "/S\0".into(),
        ] {
            assert!(!InstallerExecution::Exe { arguments }.valid());
        }
        assert!(InstallerExecution::Exe {
            arguments: "/S /D=\"C:\\Program Files\\Example\"".into()
        }
        .valid());
        for execution in [
            json!({"kind":"Msi","arguments":"/S"}),
            json!({"kind":"Exe"}),
            json!({"kind":"Shell","arguments":"echo"}),
        ] {
            assert!(serde_json::from_value::<InstallerExecution>(execution).is_err());
        }
    }

    #[cfg(windows)]
    #[test]
    fn detector_only_allows_explicit_numeric_upgrades_and_configured_version_precision() {
        let source = include_str!("deploy-installer.ps1").replace("\r\n", "\n");
        let detector=source.split("try {\n    $inputData").next().unwrap().replace("[Microsoft.Win32.RegistryKey]::OpenBaseKey([Microsoft.Win32.RegistryHive]::LocalMachine, $view)","$fixtureRoot");
        assert!(!detector.contains("New-PSSession"));
        let script = format!(
            "{detector}\n{}",
            r#"
$fixtureKey=[pscustomobject]@{}
$fixtureKey | Add-Member ScriptMethod GetValue {param($Name) $script:installedFixture}
$fixtureKey | Add-Member ScriptMethod Dispose {}
$fixtureRoot=[pscustomobject]@{}
$fixtureRoot | Add-Member ScriptMethod OpenSubKey {param($Name) if($null -ne $script:installedFixture){$fixtureKey}else{$null}}
$fixtureRoot | Add-Member ScriptMethod Dispose {}
$cases=[Console]::In.ReadToEnd() | ConvertFrom-Json
$answers=@(foreach($case in $cases){
    $script:installedFixture=$case.installed
    [string](& $detect -ProductCode '00000000-0000-0000-0000-000000000001' -Version $case.version -AllowUpgrade ([bool]$case.upgrade) -Components $case.components)
})
[Console]::Out.Write(($answers | ConvertTo-Json -Compress))
"#
        );
        let cases = json!([
            {"installed":null,"version":"2","upgrade":true},
            {"installed":"1","version":"2","upgrade":false},
            {"installed":"1.9","version":"2.0","upgrade":true},
            {"installed":"3","version":"2","upgrade":true},
            {"installed":"unknown","version":"2","upgrade":true},
            {"installed":"2.0","version":"2","upgrade":true},
            {"installed":"7.1.2609.301","version":"7.1","upgrade":false,"components":2},
            {"installed":"7.1.bad","version":"7.1","upgrade":true,"components":2},
            {"installed":"18446744073709551616","version":"2","upgrade":true}
        ]);
        let output = crate::probe::invoke(
            &script,
            Zeroizing::new(serde_json::to_vec(&cases).unwrap()),
            crate::probe::QueryLimits {
                deadline: Duration::from_secs(20),
                max_output: 4096,
                stopping: None,
            },
        )
        .unwrap();
        let actual: Vec<String> = serde_json::from_slice(&output).unwrap();
        assert_eq!(
            actual,
            [
                "Absent",
                "DifferentVersion",
                "Upgradeable",
                "DifferentVersion",
                "DifferentVersion",
                "DifferentVersion",
                "Present",
                "DifferentVersion",
                "DifferentVersion"
            ]
        );
    }

    #[cfg(windows)]
    #[test]
    fn exe_adapter_runs_a_local_harmless_fixture_without_shell_or_output_leakage() {
        // Only transport and detection are mocked. The remote process block runs locally
        // against a temporary test executable that prints data and exits; no install occurs.
        let dir = tempfile::tempdir().unwrap();
        let preamble = r#"
$fixtureJson = [Console]::In.ReadToEnd()
$fixtureData = $fixtureJson | ConvertFrom-Json
$source = 'public class Fixture { public static int Main(string[] args) { for (int i=0;i<20000;i++) { System.Console.Out.Write("OUTPUT_MUST_BE_DISCARDED"); System.Console.Error.Write("ERROR_MUST_BE_DISCARDED"); } return args.Length == 3 && args[0] == "/S" && args[1] == "value with spaces" && args[2] == "& echo SHOULD_NOT_RUN" ? 3010 : 123; } }'
Add-Type -TypeDefinition $source -OutputAssembly $fixtureData.options.path -OutputType ConsoleApplication
$fixtureData.options.sha256 = (Get-FileHash -LiteralPath $fixtureData.options.path -Algorithm SHA256).Hash
[Console]::SetIn([IO.StringReader]::new(($fixtureData | ConvertTo-Json -Depth 8 -Compress)))
$script:detections = 0
function New-PSSessionOption { param($OpenTimeout, $OperationTimeout) @{} }
function New-PSSession { param($ComputerName, $Port, [switch]$UseSSL, $Credential, $Authentication, $SessionOption) [pscustomobject]@{Fixture=$true} }
function Remove-PSSession { param($Session, $ErrorAction) }
function Copy-Item { param($LiteralPath, $Destination, $ToSession) Microsoft.PowerShell.Management\Copy-Item -LiteralPath $LiteralPath -Destination $Destination }
function Invoke-Command {
    param($Session, [scriptblock]$ScriptBlock, [object[]]$ArgumentList)
    if ($ScriptBlock.ToString().Contains('param([string]$ProductCode')) {
        $script:detections++
        $detected = if ($script:detections -eq 1) { 'Absent' } else { 'Present' }
        # Remoting decorates even scalar strings. Without explicit casting the
        # final ConvertTo-Json produces an object rather than the enum string.
        $detected | Add-Member -NotePropertyName PSComputerName -NotePropertyValue 'fixture.invalid'
        $detected
    } else { & $ScriptBlock @ArgumentList }
}
"#;
        let adapter = format!(
            "{}\n{}",
            include_str!("transfer.ps1"),
            include_str!("deploy-installer.ps1")
        )
        .replace(
            "# The boundary in $result determines whether an installer might have been launched.",
            "[Console]::Out.Write('FIXTURE_ERROR: ' + $_.Exception.Message)",
        )
        .replace(
            "[Console]::InputEncoding = [Text.UTF8Encoding]::new($false)",
            "",
        );
        let script = format!("$ErrorActionPreference='Stop'\n{preamble}\n{}", adapter);
        let input = json!({"host":"fixture.invalid", "username":"fixture", "password":"SYNTHETIC_ONLY",
        "winrm_authentication":"Negotiate", "options":{
            "path":dir.path().join("harmless.exe"), "sha256":"", "product_code":Uuid::new_v4(), "version":"1", "reconcile":false,
            "execution":{"kind":"Exe","arguments":"/S \"value with spaces\" \"& echo SHOULD_NOT_RUN\""}
        }});
        let output = crate::probe::invoke(
            &script,
            Zeroizing::new(serde_json::to_vec(&input).unwrap()),
            crate::probe::QueryLimits {
                deadline: Duration::from_secs(45),
                max_output: 4096,
                stopping: None,
            },
        )
        .unwrap();
        let result: AdapterResult = serde_json::from_slice(&output)
            .unwrap_or_else(|error| panic!("{error}: {}", String::from_utf8_lossy(&output)));
        assert!(matches!(result.outcome, Outcome::Finished));
        assert_eq!(result.exit_code, Some(3010));
        assert_eq!(result.detection, Detection::Present);
        assert!(!String::from_utf8(output)
            .unwrap()
            .contains("MUST_BE_DISCARDED"));
    }
}
