use soroban_sdk::{contracttype, Address, Bytes, String, Vec};

use crate::errors::AnalyticsError;

/// Largest span a single earnings query may cover, in ledgers.
///
/// ~30 days at 6 s/ledger. Bounding the window keeps one query from asking the
/// contract to reason about an arbitrary slice of chain history, while staying
/// well inside the retention period during which records are still live.
pub const MAX_QUERY_WINDOW_LEDGERS: u32 = 432_000;

/// Validate an inclusive ledger query window.
///
/// `from_ledger` and `to_ledger` are both included in the window. A window is
/// valid when it is not inverted (`from_ledger <= to_ledger`) and its span does
/// not exceed [`MAX_QUERY_WINDOW_LEDGERS`]. A zero-width window is valid: it
/// asks for the records of a single ledger, and legitimately yields an empty
/// result when none were recorded there.
///
/// Returns the span in ledgers so callers do not recompute it.
pub fn validate_query_window(from_ledger: u32, to_ledger: u32) -> Result<u32, AnalyticsError> {
    if to_ledger < from_ledger {
        return Err(AnalyticsError::InvalidQueryWindow);
    }
    let span = to_ledger - from_ledger;
    if span > MAX_QUERY_WINDOW_LEDGERS {
        return Err(AnalyticsError::InvalidQueryWindow);
    }
    Ok(span)
}

/// True when `ledger` falls inside the inclusive window `[from, to]`.
pub fn in_query_window(ledger: u32, from_ledger: u32, to_ledger: u32) -> bool {
    ledger >= from_ledger && ledger <= to_ledger
}

/// A single earnings record tied to a completed commission.
///
/// Tracks the category/client context for per-category and per-client
/// earnings breakdowns (issue #602).
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EarningsRecord {
    /// The artist whose earnings this record belongs to.
    pub artist: Address,
    /// Commission identifier this payout came from.
    pub commission_id: Bytes,
    /// Category label (e.g. "illustration", "ui-design").
    pub category: String,
    /// Client address associated with this commission.
    pub client: Address,
    /// Amount earned (in the platform's base token units).
    pub amount: i128,
    /// Ledger sequence when the payout was recorded.
    pub ledger: u32,
}

/// Aggregate performance metrics for a single artist.
///
/// Updated incrementally as commissions complete or fail.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ArtistMetrics {
    /// Total amount earned across all completed commissions.
    pub total_earnings: i128,
    /// Number of commissions completed successfully.
    pub completed_count: u32,
    /// Number of commissions cancelled or refunded.
    pub cancelled_count: u32,
    /// Sum of response-time ledgers (for average calculation).
    pub response_time_sum: u64,
    /// Number of data points in `response_time_sum`.
    pub response_time_count: u32,
    /// Sum of client satisfaction scores (1–5 scale, stored ×10 for precision).
    pub satisfaction_score_sum: u32,
    /// Number of satisfaction score entries.
    pub satisfaction_score_count: u32,
    /// Ledger when these metrics were last updated.
    pub last_updated_ledger: u32,
}

/// A request for one bounded page of an artist's earnings history (#47).
///
/// `limit` is a ceiling, not a promise: the contract clamps it to
/// `MAX_EARNING_PAGE` before reading, so a caller cannot turn one request into
/// an unbounded scan of the log.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EarningsPageRequest {
    pub artist: Address,
    /// Index of the first record to consider, from `get_earning_count`'s
    /// monotonic sequence.
    pub from_index: u32,
    /// Maximum number of indexes to examine.
    pub limit: u32,
}

/// One page of earnings history plus the cursor for the next page (#47).
///
/// `next_index` advances by the size of the *requested range*, not by the
/// number of records returned. Pruned indexes can therefore make a page
/// shorter than `limit` without making the next page overlap this one.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EarningsPage {
    pub records: Vec<EarningsRecord>,
    /// Cursor to pass as `from_index` for the following page. Stable even when
    /// records inside the range were pruned.
    pub next_index: u32,
    /// Total number of earnings records ever written for the artist, including
    /// records still in the retention window and pruned indexes.
    pub total: u32,
    /// Whether any index at or after `next_index` is below `total`.
    pub has_more: bool,
}

/// Storage data keys.
#[contracttype]
pub enum DataKey {
    /// Admin address.
    Admin,
    /// Per-artist aggregate metrics.  Key: artist address.
    Metrics(Address),
    /// Earnings log for an artist.  Key: (artist, index).
    Earning(Address, u32),
    /// Count of earnings entries for an artist.  Key: artist.
    EarningCount(Address),
}
