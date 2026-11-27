//! Co-located application service for M1. One bounded worker owns SQLite.
//! No egui, network, process execution or credential dependencies.
use chrono::{DateTime, Utc};
use orkestr_domain::*;
use orkestr_protocol::{
    AppliedDiscovery, ApplyDiscovery, DeviceInventoryStatus, PrepareDiscovery, PreparedDiscovery,
    DISCOVERY_TEXT_LIMIT,
};
pub use orkestr_storage::ImportReport;
use orkestr_storage::*;
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    path::PathBuf,
    sync::{
        mpsc::{self, Receiver, SyncSender, TrySendError},
        Arc,
    },
};

#[derive(Clone, Serialize, Deserialize)]
pub struct Workspace {
    pub inventory: InventorySnapshot,
    pub packages: Vec<PackageRevision>,
    pub reviews: Vec<ImportReview>,
    pub counts: BTreeMap<CollectionId, (usize, usize)>,
    pub evaluated_at: DateTime<Utc>,
}
#[derive(Clone, Serialize, Deserialize)]
pub enum Selection {
    All,
    Group(CollectionId),
    Draft(Box<Collection>),
}
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ImportFormat {
    Inventory,
    Deploy,
}

pub enum Command {
    ReadDiscoveryFile {
        token: u64,
        path: PathBuf,
    },
    PrepareDiscovery {
        token: u64,
        input: PrepareDiscovery,
    },
    ApplyDiscovery {
        token: u64,
        input: ApplyDiscovery,
    },
    DiscardDiscovery,
    InventoryStatus {
        token: u64,
        device_id: DeviceId,
    },
    Refresh,
    Preview {
        token: u64,
        selection: Selection,
    },
    Save {
        collection: Box<Collection>,
        expected: Option<u64>,
    },
    Delete {
        id: CollectionId,
        expected: u64,
    },
    PrepareImport {
        token: u64,
        format: ImportFormat,
        path: PathBuf,
        origin: Option<String>,
    },
    ApplyImport {
        token: u64,
        backup: PathBuf,
    },
    DiscardImport,
}
pub enum Event {
    DiscoveryText {
        token: u64,
        text: String,
    },
    DiscoveryPrepared {
        token: u64,
        preview: PreparedDiscovery,
    },
    DiscoveryApplied {
        token: u64,
        result: AppliedDiscovery,
    },
    InventoryStatus {
        token: u64,
        status: DeviceInventoryStatus,
    },
    Loaded(Arc<Workspace>),
    Preview {
        token: u64,
        preview: Preview,
    },
    Saved {
        id: CollectionId,
        workspace: Arc<Workspace>,
    },
    Deleted(Arc<Workspace>),
    ImportPrepared {
        token: u64,
        report: ImportReport,
    },
    Imported {
        outcome: ImportOutcome,
        workspace: Arc<Workspace>,
    },
    Error {
        operation: &'static str,
        token: Option<u64>,
        message: String,
    },
}
pub struct Service {
    sender: SyncSender<Command>,
    receiver: Receiver<Event>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SubmitError {
    Busy,
    Closed,
}
impl Service {
    /// A bounded transport worker can reuse the same UI command/event contract.
    pub fn from_transport(sender: SyncSender<Command>, receiver: Receiver<Event>) -> Self {
        Self { sender, receiver }
    }
    /// Explicitly chosen path only. create=true refuses overwrite, even under a race.
    pub fn start(path: PathBuf, create: bool, wake: Arc<dyn Fn() + Send + Sync>) -> Self {
        let (sender, commands) = mpsc::sync_channel(8);
        let (events, receiver) = mpsc::sync_channel(8);
        std::thread::spawn(move || {
            let send = |event| {
                let result = events.send(event);
                wake();
                result.is_ok()
            };
            let opened = (|| {
                if create {
                    std::fs::OpenOptions::new()
                        .write(true)
                        .create_new(true)
                        .open(&path)?;
                } else if !path.is_file() {
                    return Err(Error::Invalid(
                        "database not found; choose Create explicitly",
                    ));
                }
                Store::open(&path)
            })();
            let mut store = match opened {
                Ok(s) => s,
                Err(e) => {
                    send(failure("open", None, e));
                    return;
                }
            };
            let mut evaluation_time = match workspace(&mut store) {
                Ok(w) => {
                    let evaluated_at = w.evaluated_at;
                    if !send(Event::Loaded(Arc::new(w))) {
                        return;
                    }
                    evaluated_at
                }
                Err(e) => {
                    send(failure("load", None, e));
                    return;
                }
            };
            let mut pending: Option<(u64, ImportPlan)> = None;
            while let Ok(command) = commands.recv() {
                let event = match command {
                    Command::ReadDiscoveryFile { token, path } => read_discovery_file(&path)
                        .map(|text| Event::DiscoveryText { token, text })
                        .unwrap_or_else(|e| failure("discovery file", Some(token), e)),
                    Command::PrepareDiscovery { token, input } => store
                        .prepare_discovery_session(uuid::Uuid::nil(), &input)
                        .map(|preview| Event::DiscoveryPrepared { token, preview })
                        .unwrap_or_else(|e| failure("discovery preview", Some(token), e)),
                    Command::ApplyDiscovery { token, input } => store
                        .apply_discovery_session(uuid::Uuid::nil(), &input)
                        .map(|result| Event::DiscoveryApplied { token, result })
                        .unwrap_or_else(|e| failure("discovery apply", Some(token), e)),
                    Command::DiscardDiscovery => {
                        store.discard_discovery_session(uuid::Uuid::nil());
                        continue;
                    }
                    Command::InventoryStatus { token, device_id } => store
                        .device_inventory_status(device_id)
                        .map(|status| Event::InventoryStatus { token, status })
                        .unwrap_or_else(|e| failure("inventory status", Some(token), e)),
                    Command::Refresh => workspace(&mut store)
                        .map(|w| Event::Loaded(Arc::new(w)))
                        .unwrap_or_else(|e| failure("refresh", None, e)),
                    Command::Preview { token, selection } => {
                        preview_at(&mut store, &selection, evaluation_time)
                            .map(|preview| Event::Preview { token, preview })
                            .unwrap_or_else(|e| failure("preview", Some(token), e))
                    }
                    Command::Save {
                        collection,
                        expected,
                    } => store
                        .save_collection(&collection, expected)
                        .and_then(|c| {
                            Ok(Event::Saved {
                                id: c.id,
                                workspace: Arc::new(workspace(&mut store)?),
                            })
                        })
                        .unwrap_or_else(|e| failure("save", None, e)),
                    Command::Delete { id, expected } => store
                        .delete_collection(id, expected)
                        .and_then(|()| Ok(Event::Deleted(Arc::new(workspace(&mut store)?))))
                        .unwrap_or_else(|e| failure("delete", None, e)),
                    Command::PrepareImport {
                        token,
                        format,
                        path,
                        origin,
                    } => {
                        pending = None;
                        let plan = match format {
                            ImportFormat::Inventory => store.preview_inventory_copy(&path),
                            ImportFormat::Deploy => {
                                store.preview_deploy_copy_with_origin(&path, origin.as_deref())
                            }
                        };
                        match plan {
                            Ok(plan) => {
                                let report = plan.report().clone();
                                pending = Some((token, plan));
                                Event::ImportPrepared { token, report }
                            }
                            Err(e) => failure("import preview", Some(token), e),
                        }
                    }
                    Command::ApplyImport { token, backup } => {
                        let applied = (|| {
                            let (ticket, plan) = pending
                                .take()
                                .ok_or(Error::Invalid("prepare import again"))?;
                            if ticket != token {
                                return Err(Error::Conflict);
                            }
                            if !plan.report().can_apply() {
                                return Err(Error::Invalid("dry-run has blocking errors"));
                            }
                            store.backup(&backup)?;
                            let outcome = store.apply_import(&plan)?;
                            Ok(Event::Imported {
                                outcome,
                                workspace: Arc::new(workspace(&mut store)?),
                            })
                        })();
                        applied.unwrap_or_else(|e| failure("import apply", Some(token), e))
                    }
                    Command::DiscardImport => {
                        pending = None;
                        continue;
                    }
                };
                match &event {
                    Event::Loaded(w)
                    | Event::Saved { workspace: w, .. }
                    | Event::Deleted(w)
                    | Event::Imported { workspace: w, .. } => evaluation_time = w.evaluated_at,
                    _ => {}
                }
                if !send(event) {
                    break;
                }
            }
        });
        Self { sender, receiver }
    }
    /// Non-blocking even when the bounded worker is full.
    pub fn submit(&self, command: Command) -> std::result::Result<(), SubmitError> {
        self.sender.try_send(command).map_err(|e| match e {
            TrySendError::Full(_) => SubmitError::Busy,
            TrySendError::Disconnected(_) => SubmitError::Closed,
        })
    }
    pub fn try_event(&self) -> Option<Event> {
        self.receiver.try_recv().ok()
    }
}

/// Executed by the local OR client transport worker, never a server-side client path.
pub fn read_discovery_file(path: &std::path::Path) -> Result<String> {
    use std::io::Read;
    let name = path.to_string_lossy();
    if name.starts_with("\\\\") || name.starts_with("//") || name.contains("://") {
        return Err(Error::Invalid("choose a local regular UTF-8 file"));
    }
    let meta = std::fs::symlink_metadata(path)?;
    if !meta.file_type().is_file() || meta.len() > DISCOVERY_TEXT_LIMIT as u64 {
        return Err(Error::Invalid("choose a regular file up to 24 KiB"));
    }
    let mut bytes = vec![];
    std::fs::File::open(path)?
        .take(DISCOVERY_TEXT_LIMIT as u64 + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() > DISCOVERY_TEXT_LIMIT {
        return Err(Error::Invalid("discovery file exceeds 24 KiB"));
    }
    String::from_utf8(bytes).map_err(|_| Error::Invalid("discovery file must be UTF-8"))
}
fn failure(operation: &'static str, token: Option<u64>, error: Error) -> Event {
    Event::Error {
        operation,
        token,
        message: error.to_string(),
    }
}
pub fn workspace(store: &mut Store) -> Result<Workspace> {
    let inventory = store.snapshot()?;
    let evaluated_at = Utc::now();
    let library = store.library_references()?;
    let ctx = EvaluationContext {
        now: evaluated_at,
        max_scan_age: chrono::Duration::hours(24),
        library: &library,
    };
    let mut counts = BTreeMap::new();
    for c in &inventory.collections {
        let p = inventory.preview_collection(c.id, &ctx)?;
        counts.insert(c.id, (p.members.len(), p.unknown.len()));
    }
    Ok(Workspace {
        inventory,
        counts,
        evaluated_at,
        packages: store.packages()?,
        reviews: store.import_reviews()?,
    })
}
pub fn preview(store: &mut Store, selection: &Selection) -> Result<Preview> {
    preview_at(store, selection, Utc::now())
}
fn preview_at(store: &mut Store, selection: &Selection, now: DateTime<Utc>) -> Result<Preview> {
    let s = store.snapshot()?;
    let library = store.library_references()?;
    let ctx = EvaluationContext {
        now,
        max_scan_age: chrono::Duration::hours(24),
        library: &library,
    };
    match selection {
        Selection::All => Ok(Preview {
            generation: s.generation,
            evaluated_at: ctx.now,
            members: s.devices,
            unknown: Default::default(),
        }),
        Selection::Group(id) => Ok(s.preview_collection(*id, &ctx)?),
        Selection::Draft(c) => {
            s.validate_collection(c)?;
            let mut draft = s;
            draft.collections.retain(|old| old.id != c.id);
            draft.collections.push(c.as_ref().clone());
            Ok(draft.preview_collection(c.id, &ctx)?)
        }
    }
}

#[cfg(test)]
mod tests;
