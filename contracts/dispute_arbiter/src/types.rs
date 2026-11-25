use soroban_sdk::{contracttype, Address, Bytes, String};

#[contracttype]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DisputeStatus {
    Open = 0,
    ResolvedForClient = 1,
    ResolvedForArtist = 2,
    PartiallyResolved = 3,
    AutoResolved = 4,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DisputeRecord {
    pub commission_id: Bytes,
    pub opened_ledger: u32,
    /// The **first ledger at which the dispute window is closed** (#977, #40).
    ///
    /// One boundary, one meaning: at `deadline_ledger - 1` evidence is still
    /// accepted and the dispute cannot be auto-resolved; at `deadline_ledger`
    /// evidence is refused and `auto_resolve` is callable. The two windows are
    /// therefore adjacent, never overlapping, so no ledger can both accept
    /// evidence and settle the dispute.
    ///
    /// Set once, when the dispute is opened, from the single configurable
    /// dispute TTL ([`shared::dispute::DEFAULT_DISPUTE_TTL_LEDGERS`]) so the
    /// evidence window and the auto-resolve timeout are one concept rather
    /// than two independent timers.
    pub deadline_ledger: u32,
    pub status: DisputeStatus,
    pub resolution_note: Option<String>,
}

impl DisputeRecord {
    /// Whether evidence may still be submitted at `ledger`.
    ///
    /// True strictly before [`Self::deadline_ledger`], false from it onwards.
    pub fn evidence_window_open_at(&self, ledger: u32) -> bool {
        ledger < self.deadline_ledger
    }

    /// Whether the dispute window has closed at `ledger`.
    ///
    /// The exact complement of [`Self::evidence_window_open_at`]: true from
    /// [`Self::deadline_ledger`] onwards, which is also the first ledger at
    /// which `auto_resolve` may settle the dispute.
    pub fn window_closed_at(&self, ledger: u32) -> bool {
        ledger >= self.deadline_ledger
    }
}

/// One piece of evidence attached to a dispute (#977).
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EvidenceRecord {
    pub commission_id: Bytes,
    pub submitter: Address,
    pub evidence: Bytes,
    pub submitted_ledger: u32,
}

#[contracttype]
pub enum DataKey {
    Admin,
    EscrowContract,
    ConfigContract,
    Dispute(Bytes),
    /// Configurable dispute TTL in ledgers (#977).
    DisputeTtlLedgers,
    /// One evidence entry.  Key: (commission id, index).
    Evidence(Bytes, u32),
    /// Number of evidence entries recorded for a dispute.
    EvidenceCount(Bytes),
}
