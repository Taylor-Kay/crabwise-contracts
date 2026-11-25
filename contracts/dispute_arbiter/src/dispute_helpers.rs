//! Dispute resolution and raise_dispute handler for dispute arbiter contract.
//!
//! Resolution eligibility lives here and nowhere else (#39): a party to a
//! dispute must not be able to resolve it, and the only way a party becomes
//! eligible is by being separately authorized as an arbiter. Every resolve
//! path in this module asks [`DisputeHelper::ensure_resolver_eligible`], so the
//! rule is stated once instead of being re-derived per entry point.

use soroban_sdk::{Address, Env, Symbol, Vec};

use crate::errors::DisputeError;

/// Status code stored for an open dispute.
const STATUS_OPEN: u32 = 1;

/// Storage key prefix for a dispute's status.
fn status_key(env: &Env) -> Symbol {
    Symbol::new(env, "disp_st")
}

/// Storage key prefix for the addresses that are parties to a dispute.
fn parties_key(env: &Env) -> Symbol {
    Symbol::new(env, "disp_pty")
}

/// Storage key for the addresses explicitly authorized as arbiters.
fn arbiters_key(env: &Env) -> Symbol {
    Symbol::new(env, "disp_arb")
}

/// Helper for handling disputes in escrow agreements.
pub struct DisputeHelper;

impl DisputeHelper {
    /// Raises a dispute on a campaign/escrow milestone by participant.
    pub fn raise_dispute(env: &Env, participant: &Address, dispute_id: u64) {
        participant.require_auth();
        env.storage()
            .persistent()
            .set(&(status_key(env), dispute_id), &STATUS_OPEN);
    }

    /// Records the two sides of `dispute_id`.
    ///
    /// Parties are recorded once, when the dispute is raised, so eligibility
    /// can be decided later without trusting a caller-supplied party list.
    pub fn register_parties(env: &Env, dispute_id: u64, client: &Address, artist: &Address) {
        let mut parties = Vec::new(env);
        parties.push_back(client.clone());
        parties.push_back(artist.clone());
        env.storage()
            .persistent()
            .set(&(parties_key(env), dispute_id), &parties);
    }

    /// Whether `who` is a recorded party (client or artist) to `dispute_id`.
    pub fn is_party(env: &Env, dispute_id: u64, who: &Address) -> bool {
        let parties: Vec<Address> = env
            .storage()
            .persistent()
            .get(&(parties_key(env), dispute_id))
            .unwrap_or_else(|| Vec::new(env));
        parties.iter().any(|party| party == *who)
    }

    /// Separately authorizes `who` as an arbiter.
    ///
    /// Idempotent: authorizing an existing arbiter changes nothing, so a
    /// retried authorization cannot produce duplicate entries.
    pub fn authorize_arbiter(env: &Env, who: &Address) {
        let mut arbiters: Vec<Address> = env
            .storage()
            .persistent()
            .get(&arbiters_key(env))
            .unwrap_or_else(|| Vec::new(env));
        if !arbiters.iter().any(|arbiter| arbiter == *who) {
            arbiters.push_back(who.clone());
            env.storage().persistent().set(&arbiters_key(env), &arbiters);
        }
    }

    /// Whether `who` holds the arbiter role.
    pub fn is_arbiter(env: &Env, who: &Address) -> bool {
        let arbiters: Vec<Address> = env
            .storage()
            .persistent()
            .get(&arbiters_key(env))
            .unwrap_or_else(|| Vec::new(env));
        arbiters.iter().any(|arbiter| arbiter == *who)
    }

    /// The single resolver-eligibility rule (#39).
    ///
    /// A party cannot resolve its own dispute: only an explicitly authorized
    /// arbiter may, which is what makes "unless separately authorized as an
    /// arbiter" true. A party and a stranger are both refused with
    /// [`DisputeError::Unauthorized`], so the refusal does not reveal which
    /// addresses are parties.
    pub fn ensure_resolver_eligible(
        env: &Env,
        resolver: &Address,
        dispute_id: u64,
    ) -> Result<(), DisputeError> {
        let authorized = Self::is_arbiter(env, resolver);
        let party = Self::is_party(env, dispute_id, resolver);

        match (party, authorized) {
            // An arbiter resolves, including one that is also a party: the
            // separate authorization is the conflict-of-interest waiver.
            (_, true) => Ok(()),
            // A party may not resolve its own dispute.
            (true, false) => Err(DisputeError::Unauthorized),
            // Nor may an address that was never authorized as an arbiter.
            (false, false) => Err(DisputeError::Unauthorized),
        }
    }

    /// Resolves an open dispute by an eligible arbiter with the specified
    /// payout allocation.
    ///
    /// Eligibility is checked before the caller is asked to authorize, so a
    /// party to the dispute is refused outright rather than being allowed to
    /// sign its own resolution.
    pub fn resolve_dispute(
        env: &Env,
        resolver: &Address,
        dispute_id: u64,
        resolution_code: u32,
    ) -> Result<(), DisputeError> {
        Self::ensure_resolver_eligible(env, resolver, dispute_id)?;
        resolver.require_auth();
        env.storage()
            .persistent()
            .set(&(status_key(env), dispute_id), &resolution_code);
        Ok(())
    }

    /// The stored status code for `dispute_id`; `0` when it was never raised.
    pub fn dispute_status(env: &Env, dispute_id: u64) -> u32 {
        env.storage()
            .persistent()
            .get(&(status_key(env), dispute_id))
            .unwrap_or(0u32)
    }
}
