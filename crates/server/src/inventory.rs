use chrono::Utc;
use orkestr_domain::{DeviceId, ProviderState};
use orkestr_inventory::{
    DiscoveryBatch, InventoryReport, NormalizedInventory, ProviderReport, MAX_REPORT_BYTES,
};
use orkestr_protocol::*;
use std::{sync::atomic::AtomicBool, time::Duration};
use uuid::Uuid;
use zeroize::Zeroizing;

#[derive(serde::Deserialize)]
enum WinRmFailure {
    Unreachable,
    TlsFailed,
    AccessDenied,
    SessionFailed,
}

#[derive(serde::Deserialize)]
struct ScanResponse {
    failure: Option<WinRmFailure>,
    providers: Vec<ProviderReport>,
}

pub fn discover(
    profile: &ConnectionProfile,
    credential: &CredentialProfile,
    secret: Zeroizing<Vec<u8>>,
    base: &str,
    stopping: &AtomicBool,
) -> Result<DiscoveryBatch, &'static str> {
    let bytes = crate::probe::query(
        include_str!("directory.ps1"),
        profile,
        credential,
        secret,
        &base,
        crate::probe::QueryLimits {
            deadline: Duration::from_secs(45),
            max_output: 2 * 1024 * 1024,
            stopping: Some(stopping),
        },
    )?;
    let computers: Vec<DirectoryComputer> =
        serde_json::from_slice(&bytes).map_err(|_| "INVALID_DIRECTORY_RESPONSE")?;
    DiscoveryBatch::directory(computers).map_err(|_| "INVALID_DIRECTORY_RESPONSE")
}

pub fn scan(
    profile: &ConnectionProfile,
    credential: &CredentialProfile,
    secret: Zeroizing<Vec<u8>>,
    device_id: DeviceId,
    id: Uuid,
    stopping: &AtomicBool,
) -> orkestr_inventory::Result<(NormalizedInventory, Option<&'static str>)> {
    let started_at = Utc::now();
    let result = crate::probe::query(
        include_str!("inventory.ps1"),
        profile,
        credential,
        secret,
        &(),
        crate::probe::QueryLimits {
            deadline: Duration::from_secs(180),
            max_output: MAX_REPORT_BYTES - 1024,
            stopping: Some(stopping),
        },
    )
    .map_err(|code| {
        (
            code,
            match code {
                "CONNECTION_TEST_TIMEOUT" => ProviderFailure::Timeout,
                "SERVER_STOPPING" => ProviderFailure::Interrupted,
                _ => ProviderFailure::Unavailable,
            },
        )
    })
    .and_then(|bytes| {
        serde_json::from_slice::<ScanResponse>(&bytes)
            .map_err(|_| ("INVENTORY_INVALID_RESPONSE", ProviderFailure::InvalidData))
    });
    let failed_report = |failure| {
        vec![ProviderReport {
            provider: InventoryProvider::ComputerSystem,
            state: ProviderState::Failed,
            failure: Some(failure),
            rows: vec![],
        }]
    };
    let (providers, diagnostic) = match result {
        Ok(ScanResponse {
            failure: None,
            providers,
        }) => (providers, None),
        Ok(ScanResponse {
            failure: Some(failure),
            ..
        }) => {
            let (code, provider_failure) = match failure {
                WinRmFailure::Unreachable => {
                    ("INVENTORY_WINRM_UNREACHABLE", ProviderFailure::Unavailable)
                }
                WinRmFailure::TlsFailed => {
                    ("INVENTORY_WINRM_TLS_FAILED", ProviderFailure::Unavailable)
                }
                WinRmFailure::AccessDenied => (
                    "INVENTORY_WINRM_ACCESS_DENIED",
                    ProviderFailure::AccessDenied,
                ),
                WinRmFailure::SessionFailed => (
                    "INVENTORY_WINRM_SESSION_FAILED",
                    ProviderFailure::Unavailable,
                ),
            };
            (failed_report(provider_failure), Some(code))
        }
        Err((code, failure)) => (failed_report(failure), Some(code)),
    };
    let inventory = InventoryReport {
        id,
        device_id,
        started_at,
        finished_at: Utc::now().max(started_at),
        providers,
    }
    .normalize()?;
    Ok((inventory, diagnostic))
}
