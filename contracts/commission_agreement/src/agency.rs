//! Agency account type, artist roster and revenue-split tracking (closes #609).

use soroban_sdk::{contracttype, token, Address, Env, String, Vec};

use crate::errors::AgreementError;

pub const TOTAL_BPS: u32 = 10_000;

/// Upper bound on a single `distribute_batch` call, to keep the payout loop
/// inside a predictable resource envelope.
pub const MAX_BATCH: u32 = 25;

/// Role an address holds on an agency roster (closes #34).
///
/// The roles form an ordered ladder — `Artist` → `SeniorArtist` → `Manager` —
/// so a transition is either a promotion or a demotion of exactly one rung.
/// The role governs administrative permission on the roster; every role keeps
/// its membership and may still receive payouts.
#[contracttype]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RosterRole {
    /// A represented artist. Default for a newly rostered member.
    Artist = 0,
    /// An artist with enhanced standing on the roster.
    SeniorArtist = 1,
    /// A trusted representative allowed to administer the roster.
    Manager = 2,
}

impl RosterRole {
    /// Position on the role ladder.
    pub fn level(self) -> u32 {
        self as u32
    }

    /// Whether this role may administer roster membership.
    pub fn can_manage_roster(self) -> bool {
        matches!(self, RosterRole::Manager)
    }

    /// Every roster role may receive payouts; a role change never removes the
    /// member from the roster or its revenue split.
    pub fn can_receive_payouts(self) -> bool {
        matches!(
            self,
            RosterRole::Artist | RosterRole::SeniorArtist | RosterRole::Manager
        )
    }
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AgencyProfile {
    pub agency: Address,
    pub name: String,
    /// Split applied to a newly rostered artist unless overridden.
    pub default_split_bps: u32,
    pub artist_count: u32,
    pub created_ledger: u32,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RosterEntry {
    pub agency: Address,
    pub artist: Address,
    /// The agency's cut of this artist's gross earnings, in bps.
    pub split_bps: u32,
    pub joined_ledger: u32,
    /// Current role on the roster (closes #34).
    pub role: RosterRole,
    pub commissions: u32,
    pub gross_distributed: i128,
    pub agency_revenue: i128,
    pub artist_payouts: i128,
}

/// Rolled-up agency figures. Kept as running totals rather than recomputed from
/// the roster so a read stays O(1) as the roster grows.
#[contracttype]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct AgencyAnalytics {
    pub artist_count: u32,
    pub commissions: u32,
    pub commission_budget: i128,
    pub batches: u32,
    pub gross_distributed: i128,
    pub agency_revenue: i128,
    pub artist_payouts: i128,
}

/// One line of a batch payout: the gross owed to an artist before the agency
/// takes its rostered cut.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BatchPayment {
    pub artist: Address,
    pub gross_usdc: i128,
}

pub fn validate_split_bps(split_bps: u32) -> Result<(), AgreementError> {
    if split_bps > TOTAL_BPS {
        return Err(AgreementError::InvalidSplitBps);
    }
    Ok(())
}

/// Split a gross payment into (agency cut, artist net).
pub fn split_payment(gross_usdc: i128, split_bps: u32) -> Result<(i128, i128), AgreementError> {
    if gross_usdc <= 0 {
        return Err(AgreementError::InvalidAmount);
    }
    let agency_cut = gross_usdc
        .checked_mul(split_bps as i128)
        .ok_or(AgreementError::ArithmeticOverflow)?
        / TOTAL_BPS as i128;
    Ok((agency_cut, gross_usdc - agency_cut))
}

/// Validate a roster role change (closes #34).
///
/// Only one-rung moves are allowed: a promotion to the next level or a
/// demotion to the previous one. A no-op (`current == next`) is rejected
/// because it is not a transition, and skipping a rung in either direction is
/// rejected so permissions cannot be granted or dropped in one step.
/// Membership, the artist's split and all historical totals are untouched by
/// a valid transition — only `RosterEntry::role` changes.
pub fn validate_role_transition(
    current: RosterRole,
    next: RosterRole,
) -> Result<(), AgreementError> {
    if current == next {
        return Err(AgreementError::InvalidRoleTransition);
    }
    if current.level().abs_diff(next.level()) != 1 {
        return Err(AgreementError::InvalidRoleTransition);
    }
    Ok(())
}

/// Settle every net in a batch, or none at all (closes #35).
///
/// The caller applies the roster and analytics effects first and then settles
/// the tokens through this helper. Each leg uses `try_transfer` so a rejected
/// recipient or an insufficient agency balance becomes a typed
/// `BatchTransferFailed` instead of a trap. Returning that error fails the
/// enclosing invocation, which rolls the earlier transfers and the effects
/// back together — the batch is all-or-nothing.
pub fn settle_batch(
    env: &Env,
    token_address: &Address,
    agency: &Address,
    payments: &Vec<BatchPayment>,
    nets: &Vec<i128>,
) -> Result<(), AgreementError> {
    let token_client = token::Client::new(env, token_address);
    for (i, payment) in payments.iter().enumerate() {
        let net = nets.get(i as u32).unwrap_or(0);
        if net <= 0 {
            continue;
        }
        match token_client.try_transfer(agency, &payment.artist, &net) {
            Ok(Ok(())) => {}
            _ => return Err(AgreementError::BatchTransferFailed),
        }
    }
    Ok(())
}
