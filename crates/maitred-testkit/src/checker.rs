//! The invariant checker (ARCHITECTURE.md, Test kit: The invariant checker). It watches a test machine root and the
//! projects' records, and reports the first time it sees an invariant broken.
//!
//! Its checks are sampled: a runner calls [`Checker::observe`] on every filesystem event and on a timer, and
//! [`Checker::modified`] on every modification of a ticket, but an event stream can't count writes, so the checker
//! never claims to have seen every one. These are the invariants it can see without maitred's help; the ones that
//! need the process table (a job running before its session is in its ticket, two live jobs holding one slot) come
//! with the runner.

use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File, TryLockError};
use std::path::{Path, PathBuf};

use crate::ticket::{Read, read_ticket};

/// An invariant the checker saw broken.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Violation {
    /// A ticket disappeared before its record existed.
    GoneWithoutRecord(String),
    /// A ticket's generation went down between two reads.
    GenerationWentBack { id: String, from: u64, to: u64 },
    /// A ticket was modified while nothing held its exclusive lock.
    WrittenUnlocked(String),
    /// More than one stop marker's lock was held at once.
    TwoLiveStopMarkers(Vec<String>),
    /// At the end of the run, a ticket the checker saw has no record.
    NoRecordAtEnd(String),
}

/// Watches one machine root.
#[derive(Debug)]
pub struct Checker {
    tickets: PathBuf,
    records: Vec<PathBuf>,
    /// Each ticket present at the last look, with the newest generation read from it.
    present: BTreeMap<String, Option<u64>>,
    /// Every ticket ever seen.
    ever: BTreeSet<String>,
}

impl Checker {
    /// A checker for the machine root `root`, whose jobs' records are in each of `projects`.
    #[must_use]
    pub fn new(root: &Path, projects: &[&Path]) -> Self {
        Self {
            tickets: root.join("tickets"),
            records: projects
                .iter()
                .map(|project| project.join(".maitred/records"))
                .collect(),
            present: BTreeMap::new(),
            ever: BTreeSet::new(),
        }
    }

    /// Looks at the whole root: tickets that appeared, went, or went back a generation, and live stop markers.
    pub fn observe(&mut self) -> Vec<Violation> {
        let mut violations = Vec::new();
        let ids = self.files_ending(".tk");
        for id in &ids {
            violations.extend(self.read_generation(id));
        }
        let gone: Vec<String> = self
            .present
            .keys()
            .filter(|id| !ids.contains(*id))
            .cloned()
            .collect();
        for id in gone {
            self.present.remove(&id);
            if !self.has_record(&id) {
                violations.push(Violation::GoneWithoutRecord(id));
            }
        }
        let live: Vec<String> = self
            .files_ending(".stop")
            .into_iter()
            .filter(|id| locked(&self.tickets.join(format!("{id}.stop"))))
            .collect();
        if live.len() > 1 {
            violations.push(Violation::TwoLiveStopMarkers(live));
        }
        violations
    }

    /// Checks one ticket just reported modified: its exclusive lock must be held, unless `owner_gone(id)` says its
    /// owner died after the modification (the runner knows from its fault log and the process table). Its
    /// generation is checked too.
    pub fn modified(&mut self, id: &str, owner_gone: impl Fn(&str) -> bool) -> Vec<Violation> {
        let path = self.tickets.join(format!("{id}.tk"));
        let mut violations = Vec::new();
        if path.exists() && !locked(&path) && !owner_gone(id) {
            violations.push(Violation::WrittenUnlocked(id.into()));
        }
        violations.extend(self.read_generation(id));
        violations
    }

    /// The end of the run: every ticket ever seen must have its record.
    #[must_use]
    pub fn finish(&self) -> Vec<Violation> {
        self.ever
            .iter()
            .filter(|id| !self.has_record(id))
            .map(|id| Violation::NoRecordAtEnd(id.clone()))
            .collect()
    }

    /// Reads `id`'s ticket and checks its generation against the last one read.
    fn read_generation(&mut self, id: &str) -> Vec<Violation> {
        let generation = fs::read(self.tickets.join(format!("{id}.tk")))
            .ok()
            .and_then(|bytes| match read_ticket(&bytes) {
                Read::Valid { header, .. } => Some(header.generation),
                Read::Unknown => None,
            });
        self.ever.insert(id.into());
        let previous = self.present.insert(id.into(), generation).flatten();
        match (previous, generation) {
            (Some(from), Some(to)) if to < from => {
                vec![Violation::GenerationWentBack {
                    id: id.into(),
                    from,
                    to,
                }]
            }
            (Some(from), None) => {
                // An unreadable read keeps the last good generation, so a later read is compared with it.
                self.present.insert(id.into(), Some(from));
                vec![]
            }
            _ => vec![],
        }
    }

    /// The ids of the files in `tickets/` ending in `suffix`, ignoring `.new-*` files being created.
    fn files_ending(&self, suffix: &str) -> BTreeSet<String> {
        fs::read_dir(&self.tickets)
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|entry| entry.file_name().into_string().ok())
            .filter(|name| !name.starts_with(".new-"))
            .filter_map(|name| name.strip_suffix(suffix).map(str::to_string))
            .collect()
    }

    fn has_record(&self, id: &str) -> bool {
        self.records
            .iter()
            .any(|records| records.join(id).join("record.json").exists())
    }
}

/// Whether something holds `path`'s exclusive lock, by the shared try-lock every liveness read makes (ARCHITECTURE.md:
/// Locks that can't be fooled). The probe's own shared lock is released when the file closes.
fn locked(path: &Path) -> bool {
    File::open(path)
        .is_ok_and(|file| matches!(file.try_lock_shared(), Err(TryLockError::WouldBlock)))
}
