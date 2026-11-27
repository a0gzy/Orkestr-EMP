//! Only an explicit unattended policy can promote a verified signed update.
use orkestr_domain::{NumericVersion, PackageRevisionId};
use orkestr_protocol::*;
use orkestr_storage::{Result, Store};
use uuid::Uuid;

pub(crate) fn compatible(baseline: &ManagedInstaller, candidate: &ManagedInstaller) -> bool {
    baseline.name == candidate.name
        && baseline.execution == candidate.execution
        && baseline.allow_upgrade == candidate.allow_upgrade
        && baseline.version_components == candidate.version_components
        && baseline.registry_key == candidate.registry_key
        && baseline.product_code == candidate.product_code
}
fn metadata_version(app: &LibraryApplication, version: &str) -> Option<String> {
    NumericVersion::parse(version).ok()?;
    let compared = if let Some(count) = app.software_match.version_components {
        version
            .trim()
            .trim_start_matches('v')
            .split('.')
            .take(count as usize)
            .collect::<Vec<_>>()
            .join(".")
    } else {
        version.to_owned()
    };
    NumericVersion::parse(&compared).ok().map(|v| v.to_string())
}
pub(crate) fn signed_candidate(
    app: &LibraryApplication,
    baseline: &ManagedInstaller,
    previous: &ReleaseCandidate,
    candidate: &ReleaseCandidate,
) -> Option<ManagedInstaller> {
    if !matches!(app.verification, VerificationPolicy::Authenticode { .. })
        || !matches!(
            candidate.state,
            ReleaseState::Verified | ReleaseState::Approved
        )
        || candidate.application_revision != app.revision
        || previous.application_revision != app.revision
        || candidate.application_id != app.id
        || previous.application_id != app.id
        || NumericVersion::compare(&candidate.version, &previous.version)
            != Some(std::cmp::Ordering::Greater)
    {
        return None;
    }
    let metadata = candidate.artifact_metadata.as_ref()?;
    if !metadata.name.eq_ignore_ascii_case(&app.software_match.name)
        || !metadata
            .publisher
            .eq_ignore_ascii_case(&app.software_match.publisher)
        || metadata_version(app, &metadata.version).as_deref() != Some(candidate.version.as_str())
    {
        return None;
    }
    if metadata.architecture.as_deref() != Some(app.architecture.as_str()) {
        return None;
    }
    let mut installer = baseline.clone();
    installer.sha256 = candidate.sha256.clone()?;
    installer.version = metadata.version.clone();
    if installer.execution.is_msi() {
        installer.product_code = metadata.product_code?;
    }
    if app
        .software_match
        .product_code
        .is_some_and(|expected| expected != installer.product_code)
        || !installer.valid()
        || !compatible(baseline, &installer)
    {
        return None;
    }
    Some(installer)
}
pub(crate) fn effective_package(
    store: &mut Store,
    actor: Uuid,
    policy: &Policy,
) -> Result<PackageRevisionId> {
    let Some(baseline) = store.managed_installer(policy.package_revision_id)? else {
        return Ok(policy.package_revision_id);
    };
    let packages = store.packages()?;
    let Some(package) = packages.iter().find(|p| p.id == policy.package_revision_id) else {
        return Ok(policy.package_revision_id);
    };
    let Some(release_id) = package.metadata["release_id"]
        .as_str()
        .and_then(|s| Uuid::parse_str(s).ok())
    else {
        return Ok(policy.package_revision_id);
    };
    let Some(previous) = store.library_release(release_id)? else {
        return Ok(policy.package_revision_id);
    };
    let Some(app) = store.library_application(policy.application_id)? else {
        return Ok(policy.package_revision_id);
    };
    if previous.application_id != app.id
        || previous.application_revision != app.revision
        || !store.package_source_current(package.id)?
    {
        return Ok(policy.package_revision_id);
    }
    if policy.automation.auto_deploy && policy.automation.unattended {
        let mut releases = store.library_releases()?;
        releases.sort_by(|a, b| {
            NumericVersion::compare(&a.version, &b.version).unwrap_or(std::cmp::Ordering::Equal)
        });
        for release in releases {
            if let Some(installer) = signed_candidate(&app, &baseline, &previous, &release) {
                if packages.iter().any(|p| {
                    p.metadata["release_id"].as_str() == Some(release.id.to_string().as_str())
                        && store
                            .managed_installer(p.id)
                            .ok()
                            .flatten()
                            .is_some_and(|i| i == installer)
                }) {
                    continue;
                }
                // Exact revision/hash promotion; same-version new hashes remain manual.
                if release.state == ReleaseState::Verified
                    && store
                        .approve_release(
                            actor,
                            &ChangeRelease {
                                id: release.id,
                                application_revision: app.revision,
                            },
                            true,
                        )
                        .is_err()
                {
                    continue;
                }
                store.register_installer_origin(actor,&installer,serde_json::json!({"release_id":release.id,"application_id":app.id,"application_revision":app.revision,"policy_id":policy.id,"policy_revision":policy.revision,"automated":true}))?;
            }
        }
    }
    let mut selected = (previous.version.clone(), policy.package_revision_id);
    for package in store.packages()? {
        if package.metadata["application_id"]
            .as_str()
            .and_then(|s| Uuid::parse_str(s).ok())
            != Some(app.id.0)
            || package.metadata["application_revision"].as_u64() != Some(app.revision)
            || !store.package_source_current(package.id)?
        {
            continue;
        }
        let Some(installer) = store.managed_installer(package.id)? else {
            continue;
        };
        if !compatible(&baseline, &installer) {
            continue;
        }
        let Some(release_id) = package.metadata["release_id"]
            .as_str()
            .and_then(|s| Uuid::parse_str(s).ok())
        else {
            continue;
        };
        let Some(release) = store.library_release(release_id)? else {
            continue;
        };
        if NumericVersion::compare(&release.version, &selected.0)
            .is_some_and(|order| order != std::cmp::Ordering::Less)
        {
            selected = (release.version, package.id);
        }
    }
    Ok(selected.1)
}
pub(crate) fn override_allowed(
    store: &Store,
    policy: &Policy,
    selected: PackageRevisionId,
) -> Result<bool> {
    if selected == policy.package_revision_id {
        return Ok(true);
    }
    let (Some(baseline), Some(candidate)) = (
        store.managed_installer(policy.package_revision_id)?,
        store.managed_installer(selected)?,
    ) else {
        return Ok(false);
    };
    if !compatible(&baseline, &candidate)
        || NumericVersion::compare(&candidate.version, &baseline.version)
            .is_none_or(|order| order == std::cmp::Ordering::Less)
    {
        return Ok(false);
    }
    let packages = store.packages()?;
    let scopes: Vec<_> = [policy.package_revision_id, selected]
        .iter()
        .filter_map(|id| packages.iter().find(|p| p.id == *id))
        .collect();
    Ok(scopes.len() == 2
        && scopes.iter().all(|p| {
            p.metadata["application_id"]
                .as_str()
                .and_then(|s| Uuid::parse_str(s).ok())
                == Some(policy.application_id.0)
        })
        && scopes[0].metadata["application_revision"] == scopes[1].metadata["application_revision"])
}
