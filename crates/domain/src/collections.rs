use crate::*;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::{
    cmp::Ordering,
    collections::{BTreeMap, BTreeSet},
};

pub const GIB: u64 = 1 << 30;
const MAX_DEPTH: usize = 16;
const MAX_NODES: usize = 256;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Truth {
    Yes,
    No,
    Unknown,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Compare {
    Eq,
    Ne,
    Gt,
    Ge,
    Lt,
    Le,
}
impl Compare {
    pub fn matches(self, ordering: Ordering) -> bool {
        match self {
            Self::Eq => ordering == Ordering::Equal,
            Self::Ne => ordering != Ordering::Equal,
            Self::Gt => ordering == Ordering::Greater,
            Self::Ge => ordering != Ordering::Less,
            Self::Lt => ordering == Ordering::Less,
            Self::Le => ordering != Ordering::Greater,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum TextField {
    Host,
    Name,
    Domain,
    Ou,
    Os,
    Source,
    Site,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum TextOp {
    Equals,
    NotEquals,
    Contains,
    NotContains,
    StartsWith,
    IsEmpty,
}
impl TextOp {
    fn matches(self, actual: &str, expected: &str) -> bool {
        let (a, b) = (actual.to_lowercase(), expected.to_lowercase());
        match self {
            Self::Equals => a == b,
            Self::NotEquals => a != b,
            Self::Contains => a.contains(&b),
            Self::NotContains => !a.contains(&b),
            Self::StartsWith => a.starts_with(&b),
            Self::IsEmpty => a.is_empty(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum VersionOperand {
    Literal(String),
    /// Late-bound contract only: M4 supplies actual library reference snapshots.
    Library {
        application_id: ApplicationId,
        channel: String,
        architecture: String,
        approved: bool,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SoftwareFilter {
    pub application_id: Option<ApplicationId>,
    pub name: Option<String>,
    pub publisher: Option<String>,
    pub channel: Option<String>,
    pub architecture: Option<String>,
    pub version: Option<(Compare, VersionOperand)>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Expression {
    And(Vec<Expression>),
    Or(Vec<Expression>),
    Ram {
        op: Compare,
        bytes: u64,
    },
    RamUnknown,
    Text {
        field: TextField,
        op: TextOp,
        value: String,
    },
    /// All fields are evaluated against ONE installed-software record.
    Software(SoftwareFilter),
    /// Explicit Unknown predicate; no negation that could turn Unknown into compliant.
    IsUnknown(Box<Expression>),
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum CollectionKind {
    Folder,
    Static(BTreeSet<DeviceId>),
    Dynamic(Expression),
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Collection {
    pub id: CollectionId,
    pub parent_id: Option<CollectionId>,
    pub name: String,
    pub revision: u64,
    pub kind: CollectionKind,
}

#[derive(Clone, Debug)]
pub struct LibraryReference {
    pub installed_version_components: Option<u8>,
    pub application_id: ApplicationId,
    pub channel: String,
    pub architecture: String,
    pub approved: bool,
    pub version: String,
}

/// Every count, member list and software list is evaluated against this one snapshot.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct InventorySnapshot {
    pub generation: u64,
    pub devices: Vec<DeviceRecord>,
    pub collections: Vec<Collection>,
}

pub struct EvaluationContext<'a> {
    pub now: DateTime<Utc>,
    pub max_scan_age: chrono::Duration,
    pub library: &'a [LibraryReference],
}
impl EvaluationContext<'_> {
    fn fresh<'a>(&self, record: &'a DeviceRecord) -> Option<&'a DeviceScan> {
        record.scan.as_ref().filter(|s| {
            let age = self.now.signed_duration_since(s.scanned_at);
            age >= chrono::Duration::zero() && age <= self.max_scan_age
        })
    }
}

impl Expression {
    pub fn validate(&self) -> Result<(), DomainError> {
        fn walk(e: &Expression, depth: usize, count: &mut usize) -> Result<(), DomainError> {
            *count += 1;
            if depth > MAX_DEPTH || *count > MAX_NODES {
                return Err(DomainError::InvalidCollection("expression limit exceeded"));
            }
            match e {
                Expression::And(items) | Expression::Or(items) => {
                    if items.is_empty() {
                        return Err(DomainError::InvalidCollection("empty AND/OR"));
                    }
                    for item in items {
                        walk(item, depth + 1, count)?;
                    }
                }
                Expression::IsUnknown(item) => walk(item, depth + 1, count)?,
                Expression::Software(f) => {
                    if f.application_id.is_none()
                        && f.name.as_ref().is_none_or(|s| s.trim().is_empty())
                    {
                        return Err(DomainError::InvalidCollection("software identity required"));
                    }
                    if let Some((_, VersionOperand::Literal(v))) = &f.version {
                        NumericVersion::parse(v)?;
                    }
                    if let Some((
                        _,
                        VersionOperand::Library {
                            application_id,
                            channel,
                            architecture,
                            ..
                        },
                    )) = &f.version
                    {
                        if f.application_id != Some(*application_id)
                            || f.channel.as_deref() != Some(channel)
                            || f.architecture.as_deref() != Some(architecture)
                        {
                            return Err(DomainError::InvalidCollection(
                                "library identity/channel/architecture mismatch",
                            ));
                        }
                    }
                }
                _ => {}
            }
            Ok(())
        }
        walk(self, 0, &mut 0)
    }

    pub fn evaluate(&self, record: &DeviceRecord, context: &EvaluationContext<'_>) -> Truth {
        use Truth::*;
        match self {
            Self::And(items) | Self::Or(items) => {
                let is_and = matches!(self, Self::And(_));
                let mut unknown = false;
                for item in items {
                    match item.evaluate(record, context) {
                        No if is_and => return No,
                        Yes if !is_and => return Yes,
                        Unknown => unknown = true,
                        _ => {}
                    }
                }
                if unknown {
                    Unknown
                } else if is_and {
                    Yes
                } else {
                    No
                }
            }
            Self::Ram { op, bytes } => context
                .fresh(record)
                .and_then(|s| s.installed_ram_bytes)
                .map(|n| truth(op.matches(n.cmp(bytes))))
                .unwrap_or(Unknown),
            Self::RamUnknown => truth(
                context
                    .fresh(record)
                    .and_then(|s| s.installed_ram_bytes)
                    .is_none(),
            ),
            Self::IsUnknown(e) => truth(e.evaluate(record, context) == Unknown),
            Self::Text { field, op, value } => {
                let d = &record.device;
                let actual = match field {
                    TextField::Host => &d.host,
                    TextField::Name => &d.name,
                    TextField::Domain => &d.domain,
                    TextField::Ou => &d.ou,
                    TextField::Os => &d.os,
                    TextField::Source => &d.source,
                    TextField::Site => &d.site,
                };
                truth(op.matches(actual, value))
            }
            Self::Software(f) => {
                let Some(scan) = context.fresh(record) else {
                    return Unknown;
                };
                // A partial registry scan cannot establish absence or compliance.
                if scan.software_state != ProviderState::Complete {
                    return Unknown;
                }
                let mut unknown = false;
                for app in &scan.software {
                    if f.name.as_ref().is_some_and(|n| !text_equal(&app.name, n))
                        || f.publisher.as_ref().is_some_and(|p| {
                            !app.publisher.trim().is_empty() && !text_equal(&app.publisher, p)
                        })
                    {
                        continue;
                    }
                    if let Some(expected) = f.application_id {
                        match app.application_id {
                            Some(actual) if actual != expected => continue,
                            None => {
                                unknown = true;
                                continue;
                            }
                            _ => {}
                        }
                    }
                    // A missing registry Publisher is unknown evidence, not proof that
                    // this is a different product or that the requested app is absent.
                    if f.publisher.is_some() && app.publisher.trim().is_empty() {
                        unknown = true;
                        continue;
                    }
                    if f.channel
                        .as_ref()
                        .is_some_and(|e| app.channel.as_ref().is_some_and(|a| !text_equal(a, e)))
                        || f.architecture.as_ref().is_some_and(|e| {
                            app.architecture.as_ref().is_some_and(|a| !text_equal(a, e))
                        })
                    {
                        continue;
                    }
                    if (f.channel.is_some() && app.channel.is_none())
                        || (f.architecture.is_some() && app.architecture.is_none())
                    {
                        unknown = true;
                        continue;
                    }
                    if let Some((op, operand)) = &f.version {
                        let mut precision = None;
                        let expected = match operand {
                            VersionOperand::Literal(v) => Some(v.as_str()),
                            VersionOperand::Library {
                                application_id,
                                channel,
                                architecture,
                                approved,
                            } => {
                                let refs: Vec<_> = context
                                    .library
                                    .iter()
                                    .filter(|r| {
                                        r.application_id == *application_id
                                            && r.channel.eq_ignore_ascii_case(channel)
                                            && r.architecture.eq_ignore_ascii_case(architecture)
                                            && r.approved == *approved
                                    })
                                    .collect();
                                if refs.len() == 1 {
                                    precision = refs[0].installed_version_components;
                                    Some(refs[0].version.as_str())
                                } else {
                                    None
                                }
                            }
                        };
                        if precision.is_some() && NumericVersion::parse(&app.version).is_err() {
                            unknown = true;
                            continue;
                        }
                        let installed = if let Some(count) = precision {
                            app.version
                                .trim()
                                .trim_start_matches('v')
                                .split('.')
                                .take(count as usize)
                                .collect::<Vec<_>>()
                                .join(".")
                        } else {
                            app.version.clone()
                        };
                        match expected.and_then(|v| NumericVersion::compare(&installed, v)) {
                            Some(order) if op.matches(order) => return Yes,
                            None => unknown = true,
                            _ => {}
                        }
                    } else {
                        return Yes;
                    }
                }
                if unknown {
                    Unknown
                } else {
                    No
                }
            }
        }
    }
}
fn truth(value: bool) -> Truth {
    if value {
        Truth::Yes
    } else {
        Truth::No
    }
}
fn text_equal(actual: &str, expected: &str) -> bool {
    actual.to_lowercase() == expected.to_lowercase()
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Preview {
    pub generation: u64,
    pub evaluated_at: DateTime<Utc>,
    pub members: Vec<DeviceRecord>,
    pub unknown: BTreeSet<DeviceId>,
}

impl InventorySnapshot {
    pub fn validate_collection(&self, incoming: &Collection) -> Result<(), DomainError> {
        if incoming.name.trim().is_empty() || incoming.name.len() > 256 {
            return Err(DomainError::InvalidCollection(
                "name required; max 256 bytes",
            ));
        }
        if let CollectionKind::Dynamic(e) = &incoming.kind {
            e.validate()?;
        }
        if let CollectionKind::Static(members) = &incoming.kind {
            if members
                .iter()
                .any(|id| !self.devices.iter().any(|d| d.device.id == *id))
            {
                return Err(DomainError::InvalidCollection("unknown static member"));
            }
        }
        if !matches!(incoming.kind, CollectionKind::Folder)
            && self
                .collections
                .iter()
                .any(|c| c.parent_id == Some(incoming.id))
        {
            return Err(DomainError::InvalidCollection(
                "move children before converting folder",
            ));
        }
        let mut seen = BTreeSet::from([incoming.id]);
        let mut next = incoming.parent_id;
        while let Some(id) = next {
            if !seen.insert(id) {
                return Err(DomainError::InvalidCollection("parent cycle"));
            }
            let parent = self
                .collections
                .iter()
                .find(|c| c.id == id)
                .ok_or(DomainError::InvalidCollection("parent not found"))?;
            if !matches!(parent.kind, CollectionKind::Folder) {
                return Err(DomainError::InvalidCollection("parent must be a folder"));
            }
            next = parent.parent_id;
        }
        Ok(())
    }

    pub fn preview_expression(
        &self,
        expression: &Expression,
        context: &EvaluationContext<'_>,
    ) -> Result<Preview, DomainError> {
        expression.validate()?;
        let mut result = Preview {
            generation: self.generation,
            evaluated_at: context.now,
            members: vec![],
            unknown: BTreeSet::new(),
        };
        for record in &self.devices {
            match expression.evaluate(record, context) {
                Truth::Yes => result.members.push(record.clone()),
                Truth::Unknown => {
                    result.unknown.insert(record.device.id);
                }
                Truth::No => {}
            }
        }
        Ok(result)
    }

    pub fn preview_collection(
        &self,
        id: CollectionId,
        context: &EvaluationContext<'_>,
    ) -> Result<Preview, DomainError> {
        // Iterative traversal: deep user-created folder trees cannot overflow the stack.
        let (mut yes, mut unknown) = (BTreeSet::new(), BTreeSet::new());
        let mut pending = vec![id];
        let mut visited = BTreeSet::new();
        while let Some(id) = pending.pop() {
            if !visited.insert(id) {
                return Err(DomainError::InvalidCollection("parent cycle"));
            }
            let c = self
                .collections
                .iter()
                .find(|c| c.id == id)
                .ok_or(DomainError::InvalidCollection("not found"))?;
            match &c.kind {
                CollectionKind::Folder => {
                    for child in self.collections.iter().filter(|c| c.parent_id == Some(id)) {
                        pending.push(child.id);
                    }
                }
                CollectionKind::Static(members) => yes.extend(members),
                CollectionKind::Dynamic(e) => {
                    let p = self.preview_expression(e, context)?;
                    yes.extend(p.members.iter().map(|r| r.device.id));
                    unknown.extend(p.unknown);
                }
            }
        }
        unknown.retain(|id| !yes.contains(id));
        Ok(Preview {
            generation: self.generation,
            evaluated_at: context.now,
            members: self
                .devices
                .iter()
                .filter(|r| yes.contains(&r.device.id))
                .cloned()
                .collect(),
            unknown,
        })
    }

    pub fn paths(&self) -> Result<BTreeMap<CollectionId, String>, DomainError> {
        let mut paths = BTreeMap::new();
        for c in &self.collections {
            let (mut names, mut seen, mut next) = (vec![], BTreeSet::new(), Some(c.id));
            while let Some(id) = next {
                if !seen.insert(id) {
                    return Err(DomainError::InvalidCollection("parent cycle"));
                }
                let node = self
                    .collections
                    .iter()
                    .find(|n| n.id == id)
                    .ok_or(DomainError::InvalidCollection("parent not found"))?;
                names.push(node.name.as_str());
                next = node.parent_id;
            }
            names.reverse();
            paths.insert(c.id, names.join(" / "));
        }
        Ok(paths)
    }
}
