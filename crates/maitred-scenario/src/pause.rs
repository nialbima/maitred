//! maitred's pause points (ARCHITECTURE.md, Test kit: "The pause protocol"). The doc's list is the authority: a
//! point is added there first, then here, so a scenario can only name a point maitred has.

/// A named place in maitred's debug builds where a process can be held while the test kit acts.
///
/// Each sits after the state it names is durable, so a KILL there leaves exactly that state.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum PausePoint {
    AdmitLocked,
    AdmitDecided,
    TicketWritten,
    TicketSynced,
    TicketRenamed,
    HolderParked,
    HolderForked,
    StopMarked,
    RecordWritten,
}

impl PausePoint {
    /// Every point, in the doc's order.
    pub const ALL: [Self; 9] = [
        Self::AdmitLocked,
        Self::AdmitDecided,
        Self::TicketWritten,
        Self::TicketSynced,
        Self::TicketRenamed,
        Self::HolderParked,
        Self::HolderForked,
        Self::StopMarked,
        Self::RecordWritten,
    ];

    /// The point's dotted name, as the protocol writes it.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::AdmitLocked => "admit.locked",
            Self::AdmitDecided => "admit.decided",
            Self::TicketWritten => "ticket.written",
            Self::TicketSynced => "ticket.synced",
            Self::TicketRenamed => "ticket.renamed",
            Self::HolderParked => "holder.parked",
            Self::HolderForked => "holder.forked",
            Self::StopMarked => "stop.marked",
            Self::RecordWritten => "record.written",
        }
    }

    /// The point with this dotted name, if the doc lists one.
    #[must_use]
    pub fn named(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|point| point.name() == name)
    }
}
