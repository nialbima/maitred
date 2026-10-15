//! What a scenario does to a run from outside: the faults of ARCHITECTURE.md, Test kit: Faults.
//!
//! The simulator models each one; the real-process runner carries each one out for real, at a pause point
//! where the fault names one.

/// Which of a job's maitred processes a fault acts on.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Process {
    Supervisor,
    Holder,
}

/// The signal a fault sends.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Signal {
    Term,
    Int,
    Hup,
    Kill,
    Stop,
    Cont,
}

/// One fault, aimed at a job by its label.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Fault {
    /// Sends a signal to one of the job's maitred processes.
    Signal {
        label: String,
        process: Process,
        signal: Signal,
    },
    /// Deletes the job's ticket file underneath it.
    DeleteTicket { label: String },
    /// Deletes the job's request directory underneath it.
    DeleteRequestDir { label: String },
    /// Deletes the job's stop marker, if it has one.
    DeleteStopMarker { label: String },
    /// Writes a cancel request for the job, as `maitred cancel` would.
    Cancel { label: String, reason: String },
}

impl Fault {
    /// Sends `signal` to the job's `process`.
    #[must_use]
    pub fn signal(label: &str, process: Process, signal: Signal) -> Self {
        Self::Signal {
            label: label.into(),
            process,
            signal,
        }
    }

    /// Deletes the job's ticket file.
    #[must_use]
    pub fn delete_ticket(label: &str) -> Self {
        Self::DeleteTicket {
            label: label.into(),
        }
    }

    /// Deletes the job's request directory.
    #[must_use]
    pub fn delete_request_dir(label: &str) -> Self {
        Self::DeleteRequestDir {
            label: label.into(),
        }
    }

    /// Deletes the job's stop marker.
    #[must_use]
    pub fn delete_stop_marker(label: &str) -> Self {
        Self::DeleteStopMarker {
            label: label.into(),
        }
    }

    /// Cancels the job with a reason.
    #[must_use]
    pub fn cancel(label: &str, reason: &str) -> Self {
        Self::Cancel {
            label: label.into(),
            reason: reason.into(),
        }
    }

    /// The job the fault acts on.
    #[must_use]
    pub fn label(&self) -> &str {
        match self {
            Self::Signal { label, .. }
            | Self::DeleteTicket { label }
            | Self::DeleteRequestDir { label }
            | Self::DeleteStopMarker { label }
            | Self::Cancel { label, .. } => label,
        }
    }
}
