extern crate std;
use std::format;

use soroban_sdk::{
    testutils::{Address as _, Events as _, Ledger as _},
    Address, Bytes, Env, String,
};

use crate::types::{ArtistMetrics, DataKey};
use crate::{AnalyticsContract, AnalyticsContractClient, errors::AnalyticsError};

fn setup(env: &Env) -> (AnalyticsContractClient<'_>, Address) {
    let contract_id = env.register_contract(None, AnalyticsContract);
    let ttl = crate::ANALYTICS_TTL_LEDGERS * 3;
    env.as_contract(&contract_id, || {
        env.storage().instance().extend_ttl(ttl, ttl);
    });
    let client = AnalyticsContractClient::new(env, &contract_id);
    let admin = Address::generate(env);
    client.initialize(&admin);
    env.as_contract(&contract_id, || {
        env.storage().instance().extend_ttl(ttl, ttl);
    });
    (client, admin)
}

fn make_bytes(env: &Env, s: &str) -> Bytes {
    Bytes::from_slice(env, s.as_bytes())
}

fn make_string(env: &Env, s: &str) -> String {
    String::from_str(env, s)
}

// ── Initialization ─────────────────────────────────────────────────────────

#[test]
fn test_initialize_succeeds() {
    let env = Env::default();
    env.mock_all_auths();
    let (_, _admin) = setup(&env);
    // If we reach here without panicking, initialization succeeded.
}

#[test]
fn test_double_initialize_fails() {
    let env = Env::default();
    env.mock_all_auths();
    let contract_id = env.register_contract(None, AnalyticsContract);
    let client = AnalyticsContractClient::new(&env, &contract_id);
    let admin = Address::generate(&env);
    client.initialize(&admin);
    let result = client.try_initialize(&admin);
    assert_eq!(result, Err(Ok(AnalyticsError::AlreadyInitialized)));
}

// ── Earnings recording ─────────────────────────────────────────────────────

#[test]
fn test_record_earning_increments_totals() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin) = setup(&env);

    let artist = Address::generate(&env);
    let client_addr = Address::generate(&env);
    let commission_id = make_bytes(&env, "comm-1");
    let category = make_string(&env, "illustration");

    client.record_earning(&artist, &commission_id, &category, &client_addr, &5_000);

    let metrics = client.get_metrics(&artist);
    assert_eq!(metrics.total_earnings, 5_000);
    assert_eq!(metrics.completed_count, 1);
    assert_eq!(metrics.cancelled_count, 0);
}

#[test]
fn test_record_earning_accumulates() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin) = setup(&env);

    let artist = Address::generate(&env);
    let client_addr = Address::generate(&env);
    let cat = make_string(&env, "design");

    client.record_earning(&artist, &make_bytes(&env, "c1"), &cat, &client_addr, &3_000);
    client.record_earning(&artist, &make_bytes(&env, "c2"), &cat, &client_addr, &7_000);

    let metrics = client.get_metrics(&artist);
    assert_eq!(metrics.total_earnings, 10_000);
    assert_eq!(metrics.completed_count, 2);
}

#[test]
fn test_record_earning_zero_amount_fails() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin) = setup(&env);

    let artist = Address::generate(&env);
    let client_addr = Address::generate(&env);
    let cat = make_string(&env, "design");
    let result = client.try_record_earning(
        &artist, &make_bytes(&env, "c1"), &cat, &client_addr, &0,
    );
    assert_eq!(result, Err(Ok(AnalyticsError::InvalidAmount)));
}

#[test]
fn test_earning_log_stored_and_retrievable() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin) = setup(&env);

    let artist = Address::generate(&env);
    let client_addr = Address::generate(&env);
    let category = make_string(&env, "animation");
    let commission_id = make_bytes(&env, "anim-1");

    client.record_earning(&artist, &commission_id, &category, &client_addr, &9_000);

    assert_eq!(client.get_earning_count(&artist), 1);
    let rec = client.get_earning(&artist, &0);
    assert_eq!(rec.amount, 9_000);
    assert_eq!(rec.artist, artist);
}

// ── Cancellation ───────────────────────────────────────────────────────────

#[test]
fn test_record_cancellation_increments_count() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin) = setup(&env);

    let artist = Address::generate(&env);
    client.record_cancellation(&artist);

    let metrics = client.get_metrics(&artist);
    assert_eq!(metrics.cancelled_count, 1);
    assert_eq!(metrics.completed_count, 0);
}

// ── Completion rate ────────────────────────────────────────────────────────

#[test]
fn test_completion_rate_100_percent() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin) = setup(&env);

    let artist = Address::generate(&env);
    let client_addr = Address::generate(&env);
    let cat = make_string(&env, "photo");

    client.record_earning(&artist, &make_bytes(&env, "p1"), &cat, &client_addr, &1_000);
    client.record_earning(&artist, &make_bytes(&env, "p2"), &cat, &client_addr, &2_000);

    assert_eq!(client.get_completion_rate(&artist), 100);
}

#[test]
fn test_completion_rate_50_percent() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin) = setup(&env);

    let artist = Address::generate(&env);
    let client_addr = Address::generate(&env);
    let cat = make_string(&env, "photo");

    client.record_earning(&artist, &make_bytes(&env, "p1"), &cat, &client_addr, &1_000);
    client.record_cancellation(&artist);

    assert_eq!(client.get_completion_rate(&artist), 50);
}

#[test]
fn test_completion_rate_no_data_returns_zero() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin) = setup(&env);

    let artist = Address::generate(&env);
    // artist never registered – should be NotFound
    let result = client.try_get_completion_rate(&artist);
    assert_eq!(result, Err(Ok(AnalyticsError::NotFound)));
}

// ── Response time ──────────────────────────────────────────────────────────

#[test]
fn test_record_response_time_and_avg() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin) = setup(&env);

    let artist = Address::generate(&env);
    client.record_response_time(&artist, &100u64);
    client.record_response_time(&artist, &200u64);

    assert_eq!(client.get_avg_response_time(&artist), 150);
}

#[test]
fn test_record_response_time_zero_fails() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin) = setup(&env);

    let artist = Address::generate(&env);
    let result = client.try_record_response_time(&artist, &0u64);
    assert_eq!(result, Err(Ok(AnalyticsError::InvalidAmount)));
}

// ── Satisfaction ───────────────────────────────────────────────────────────

#[test]
fn test_record_satisfaction_and_avg() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin) = setup(&env);

    let artist = Address::generate(&env);
    client.record_satisfaction(&artist, &40u32); // 4.0
    client.record_satisfaction(&artist, &50u32); // 5.0

    assert_eq!(client.get_avg_satisfaction(&artist), 45); // 4.5 × 10
}

#[test]
fn test_satisfaction_score_too_low_fails() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin) = setup(&env);
    let artist = Address::generate(&env);
    let result = client.try_record_satisfaction(&artist, &5u32);
    assert_eq!(result, Err(Ok(AnalyticsError::InvalidScore)));
}

#[test]
fn test_satisfaction_score_too_high_fails() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin) = setup(&env);
    let artist = Address::generate(&env);
    let result = client.try_record_satisfaction(&artist, &55u32);
    assert_eq!(result, Err(Ok(AnalyticsError::InvalidScore)));
}

#[test]
fn test_satisfaction_boundary_values_ok() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin) = setup(&env);
    let artist = Address::generate(&env);
    client.record_satisfaction(&artist, &10u32); // minimum valid
    client.record_satisfaction(&artist, &50u32); // maximum valid
    assert_eq!(client.get_avg_satisfaction(&artist), 30);
}

// ── Earnings prediction ────────────────────────────────────────────────────

#[test]
fn test_predict_earnings_average_payout() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin) = setup(&env);

    let artist = Address::generate(&env);
    let client_addr = Address::generate(&env);
    let cat = make_string(&env, "ui");

    client.record_earning(&artist, &make_bytes(&env, "u1"), &cat, &client_addr, &6_000);
    client.record_earning(&artist, &make_bytes(&env, "u2"), &cat, &client_addr, &4_000);

    // Mean = 10_000 / 2 = 5_000
    assert_eq!(client.predict_earnings(&artist), 5_000);
}

#[test]
fn test_predict_earnings_no_data_returns_zero() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin) = setup(&env);

    let artist = Address::generate(&env);
    let result = client.try_predict_earnings(&artist);
    assert_eq!(result, Err(Ok(AnalyticsError::NotFound)));
}

// ── Not found ─────────────────────────────────────────────────────────────

#[test]
fn test_get_metrics_not_found() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin) = setup(&env);
    let artist = Address::generate(&env);
    let result = client.try_get_metrics(&artist);
    assert_eq!(result, Err(Ok(AnalyticsError::NotFound)));
}

// ── Absent data vs a real zero (#46) ──────────────────────────────────────

fn zero_metrics() -> ArtistMetrics {
    ArtistMetrics {
        total_earnings: 0,
        completed_count: 0,
        cancelled_count: 0,
        response_time_sum: 0,
        response_time_count: 0,
        satisfaction_score_sum: 0,
        satisfaction_score_count: 0,
        last_updated_ledger: 0,
    }
}

fn store_metrics(
    env: &Env,
    client: &AnalyticsContractClient,
    artist: &Address,
    metrics: &ArtistMetrics,
) {
    env.as_contract(&client.address, || {
        env.storage()
            .persistent()
            .set(&DataKey::Metrics(artist.clone()), metrics);
    });
}

#[test]
fn test_absent_metrics_report_not_found_not_zero() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin) = setup(&env);

    // No record at all: every query says "absent", none of them says "0".
    let artist = Address::generate(&env);
    assert_eq!(
        client.try_get_metrics(&artist),
        Err(Ok(AnalyticsError::NotFound))
    );
    assert_eq!(
        client.try_get_completion_rate(&artist),
        Err(Ok(AnalyticsError::NotFound))
    );
    assert_eq!(
        client.try_get_avg_response_time(&artist),
        Err(Ok(AnalyticsError::NotFound))
    );
    assert_eq!(
        client.try_get_avg_satisfaction(&artist),
        Err(Ok(AnalyticsError::NotFound))
    );
    assert_eq!(
        client.try_predict_earnings(&artist),
        Err(Ok(AnalyticsError::NotFound))
    );
}

#[test]
fn test_real_zero_is_distinct_from_absent_data() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin) = setup(&env);

    // An artist with a record but no completed commissions and no response
    // times: the derived values are genuine zeros.
    let artist = Address::generate(&env);
    client.record_satisfaction(&artist, &50u32);

    assert!(client.try_get_metrics(&artist).is_ok());
    assert_eq!(client.get_completion_rate(&artist), 0);
    assert_eq!(client.get_avg_response_time(&artist), 0);
    assert_eq!(client.predict_earnings(&artist), 0);

    // The same query on an artist with no record is absence, not zero.
    let stranger = Address::generate(&env);
    assert_eq!(
        client.try_get_completion_rate(&stranger),
        Err(Ok(AnalyticsError::NotFound))
    );
}

#[test]
fn test_inconsistent_metric_data_is_rejected_as_invalid() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin) = setup(&env);

    // A maintained sum with a zero sample count cannot be averaged.
    let bad_satisfaction = Address::generate(&env);
    let mut metrics = zero_metrics();
    metrics.satisfaction_score_sum = 50;
    store_metrics(&env, &client, &bad_satisfaction, &metrics);

    // Earnings without a completed commission contradict each other.
    let bad_earnings = Address::generate(&env);
    let mut metrics = zero_metrics();
    metrics.total_earnings = 1_000;
    store_metrics(&env, &client, &bad_earnings, &metrics);

    for artist in [bad_satisfaction, bad_earnings] {
        assert_eq!(
            client.try_get_metrics(&artist),
            Err(Ok(AnalyticsError::InvalidMetricData))
        );
        assert_eq!(
            client.try_get_avg_satisfaction(&artist),
            Err(Ok(AnalyticsError::InvalidMetricData))
        );
        assert_eq!(
            client.try_get_completion_rate(&artist),
            Err(Ok(AnalyticsError::InvalidMetricData))
        );
        assert_eq!(
            client.try_predict_earnings(&artist),
            Err(Ok(AnalyticsError::InvalidMetricData))
        );
    }
}

// ── Bounded page reads (#650) ───────────────────────────────────────────────

#[test]
fn test_get_earnings_returns_bounded_page() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin) = setup(&env);

    let artist = Address::generate(&env);
    let client_addr = Address::generate(&env);
    let cat = make_string(&env, "ui");

    // Distinct amounts so page ordering is observable.
    for i in 1..=5u32 {
        client.record_earning(
            &artist,
            &make_bytes(&env, &format!("c{}", i)),
            &cat,
            &client_addr,
            &(i as i128 * 100),
        );
    }

    let first = client.get_earnings(&artist, &0, &2);
    assert_eq!(first.len(), 2, "limit must bound the page size");
    assert_eq!(first.get(0).unwrap().amount, 100);
    assert_eq!(first.get(1).unwrap().amount, 200);

    let second = client.get_earnings(&artist, &2, &2);
    assert_eq!(second.len(), 2);
    assert_eq!(second.get(0).unwrap().amount, 300);
    assert_eq!(second.get(1).unwrap().amount, 400);

    // Past the end is an empty page, not an error, so a client can page to the
    // end without first reading get_earning_count.
    assert_eq!(client.get_earnings(&artist, &99, &2).len(), 0);

    // A limit of 0 is an empty page rather than an unbounded read.
    assert_eq!(client.get_earnings(&artist, &0, &0).len(), 0);
}

// ── Query windows (#45) ─────────────────────────────────────────────────────

/// An inverted window is rejected, not silently normalised.
#[test]
fn test_query_window_rejects_inverted_range() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin) = setup(&env);
    let artist = Address::generate(&env);

    assert_eq!(
        client.try_get_earnings_in_window(&artist, &200, &100, &0, &10),
        Err(Ok(AnalyticsError::InvalidQueryWindow)),
    );
}

/// A window wider than the documented maximum is rejected, while a window of
/// exactly the maximum span is accepted.
#[test]
fn test_query_window_enforces_maximum_span() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin) = setup(&env);
    let artist = Address::generate(&env);
    let max = crate::types::MAX_QUERY_WINDOW_LEDGERS;

    assert_eq!(
        client.try_get_earnings_in_window(&artist, &0, &(max + 1), &0, &10),
        Err(Ok(AnalyticsError::InvalidQueryWindow)),
    );
    assert!(
        client.try_get_earnings_in_window(&artist, &0, &max, &0, &10).is_ok(),
        "a window of exactly the maximum span must be accepted",
    );
}

/// A normal window returns only the records recorded inside it.
#[test]
fn test_query_window_returns_only_records_inside_it() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin) = setup(&env);

    let artist = Address::generate(&env);
    let client_addr = Address::generate(&env);
    let cat = make_string(&env, "ui");
    let record_at = |ledger: u32, amount: i128| {
        set_ledger_seq(&env, &client, ledger);
        client.record_earning(
            &artist,
            &make_bytes(&env, "c"),
            &cat,
            &client_addr,
            &amount,
        );
    };
    record_at(100, 100);
    record_at(200, 200);
    record_at(300, 300);

    let inside = client.get_earnings_in_window(&artist, &150, &250, &0, &10);
    assert_eq!(inside.len(), 1, "only the ledger-200 record is in range");
    assert_eq!(inside.get(0).unwrap().amount, 200);

    // Both bounds are inclusive.
    let edges = client.get_earnings_in_window(&artist, &100, &300, &0, &10);
    assert_eq!(edges.len(), 3);

    // The page bound still applies, and paging continues past skipped records.
    let first_page = client.get_earnings_in_window(&artist, &100, &300, &0, &2);
    assert_eq!(first_page.len(), 2);
    let second_page = client.get_earnings_in_window(&artist, &100, &300, &2, &2);
    assert_eq!(second_page.len(), 1);
    assert_eq!(second_page.get(0).unwrap().amount, 300);
}

/// A valid window with no records — including a zero-width window — is an
/// empty result, not an error.
#[test]
fn test_query_window_with_no_records_is_empty() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin) = setup(&env);

    let artist = Address::generate(&env);
    set_ledger_seq(&env, &client, ledger_inside_window());
    seed_earnings(&env, &client, &artist, 1);
    let recorded_at = ledger_inside_window();

    // An artist with history, queried in a period that holds none of it.
    let quiet = client.get_earnings_in_window(&artist, &(recorded_at + 1), &(recorded_at + 100), &0, &10);
    assert_eq!(quiet.len(), 0);

    // A single-ledger window is valid even when nothing was recorded there.
    let point = client.get_earnings_in_window(&artist, &(recorded_at + 1), &(recorded_at + 1), &0, &10);
    assert_eq!(point.len(), 0);

    // An artist with no earnings at all is empty too.
    let unknown = Address::generate(&env);
    let none = client.get_earnings_in_window(&unknown, &0, &100, &0, &10);
    assert_eq!(none.len(), 0);
}

// ── Retention / pruning (#651) ──────────────────────────────────────────────

/// Record `n` earnings of 1_000 each for `artist`, each carrying a distinct
/// `commission_id` so a record can be looked up individually.
///
/// The caller positions the ledger first: `EarningsRecord.ledger` is stamped
/// from `env.ledger().sequence()`, and that stamp is what the retention check
/// keys off.
fn seed_earnings(env: &Env, client: &AnalyticsContractClient, artist: &Address, n: u32) {
    assert!(n > 0, "seed_earnings needs at least one record");
    let client_addr = Address::generate(env);
    let cat = make_string(env, "seed");
    for i in 0..n {
        client.record_earning(
            artist,
            &make_bytes(env, &format!("seed-{}", i)),
            &cat,
            &client_addr,
            &1_000,
        );
    }
}

fn set_ledger_seq(env: &Env, client: &AnalyticsContractClient, seq: u32) {
    let ttl = crate::ANALYTICS_TTL_LEDGERS * 3;
    env.as_contract(&client.address, || {
        env.storage().instance().extend_ttl(ttl, ttl);
    });
    env.ledger().set_sequence_number(seq);
    env.as_contract(&client.address, || {
        env.storage().instance().extend_ttl(ttl, ttl);
    });
}

/// Move the ledger forward far enough that every entry recorded at
/// `recorded_at` has aged out of the retention window.
fn age_out(env: &Env, client: &AnalyticsContractClient, recorded_at: u32) {
    set_ledger_seq(env, client, recorded_at.saturating_add(crate::ANALYTICS_TTL_LEDGERS));
}

/// A ledger sequence at which an entry recorded *now* is still comfortably
/// inside the retention window, so a prune at this ledger must not touch it.
///
/// The default `Env` starts at ledger 0, where `cutoff` saturates to 0 and a
/// record stamped at 0 would be immediately prunable. Every prune test
/// therefore positions the ledger explicitly first.
fn ledger_inside_window() -> u32 {
    crate::ANALYTICS_TTL_LEDGERS + 100
}

#[test]
fn test_prune_earnings_respects_retention_window() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin) = setup(&env);

    // The view entry point reports the same numbers the implementation uses.
    let (retention, batch) = client.get_retention_policy();
    assert_eq!(retention, crate::ANALYTICS_TTL_LEDGERS);
    assert_eq!(batch, crate::PRUNE_MAX_BATCH);

    let artist = Address::generate(&env);
    set_ledger_seq(&env, &client, ledger_inside_window());
    seed_earnings(&env, &client, &artist, 3);

    // Every record is inside the window, so nothing may be removed — not even
    // with the widest window and the largest batch the implementation allows.
    assert_eq!(client.prune_earnings(&artist, &10, &batch), 0);
    assert_eq!(client.get_earning_count(&artist), 3);
    assert!(client.try_get_earning(&artist, &0).is_ok());

    // Age the records out, then the same call succeeds.
    age_out(&env, &client, ledger_inside_window());
    assert_eq!(client.prune_earnings(&artist, &10, &batch), 3);

    // Records are gone, but the monotonic count is intentionally not lowered —
    // decrementing it would let a later record_earning reuse a live index.
    assert_eq!(
        client.try_get_earning(&artist, &0),
        Err(Ok(AnalyticsError::NotFound)),
    );
    assert_eq!(client.get_earning_count(&artist), 3);
}

#[test]
fn test_prune_earnings_is_bounded_by_limit() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin) = setup(&env);

    let artist = Address::generate(&env);
    set_ledger_seq(&env, &client, ledger_inside_window());
    seed_earnings(&env, &client, &artist, 6);
    age_out(&env, &client, ledger_inside_window());

    // One call removes at most `limit` records; progress is made by repetition.
    assert_eq!(client.prune_earnings(&artist, &6, &2), 2);
    assert_eq!(client.prune_earnings(&artist, &6, &2), 2);
    assert_eq!(client.prune_earnings(&artist, &6, &2), 2);
    // Nothing left to remove.
    assert_eq!(client.prune_earnings(&artist, &6, &2), 0);

    // The lifetime aggregate is never pruned.
    let metrics = client.get_metrics(&artist);
    assert_eq!(metrics.completed_count, 6);
    assert_eq!(metrics.total_earnings, 6_000);
}

#[test]
fn test_prune_earnings_rejects_out_of_range_arguments() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin) = setup(&env);

    let artist = Address::generate(&env);
    set_ledger_seq(&env, &client, ledger_inside_window());
    seed_earnings(&env, &client, &artist, 2);
    age_out(&env, &client, ledger_inside_window());

    let (_retention, batch) = client.get_retention_policy();

    // A zero window is meaningless.
    assert_eq!(
        client.try_prune_earnings(&artist, &0, &10),
        Err(Ok(AnalyticsError::InvalidAmount)),
    );
    // A zero limit would be an unbounded-looking call.
    assert_eq!(
        client.try_prune_earnings(&artist, &10, &0),
        Err(Ok(AnalyticsError::InvalidAmount)),
    );
    // A limit above the batch cap is rejected outright rather than clamped, so
    // an operator asking for a huge sweep gets an error instead of a surprise.
    assert_eq!(
        client.try_prune_earnings(&artist, &10, &(batch + 1)),
        Err(Ok(AnalyticsError::InvalidAmount)),
    );
    // Rejected calls remove nothing.
    assert!(client.try_get_earning(&artist, &0).is_ok());
    assert_eq!(client.get_earning_count(&artist), 2);
}

#[test]
fn test_prune_earnings_stops_at_a_protected_record() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin) = setup(&env);

    let artist = Address::generate(&env);
    set_ledger_seq(&env, &client, ledger_inside_window());
    seed_earnings(&env, &client, &artist, 2);
    age_out(&env, &client, ledger_inside_window());

    // One record written *after* ageing — so it is inside the retention window.
    let client_addr = Address::generate(&env);
    let cat = make_string(&env, "recent");
    client.record_earning(
        &artist,
        &make_bytes(&env, "recent"),
        &cat,
        &client_addr,
        &1_000,
    );

    // A wide window and a wide batch must still stop at the protected record
    // rather than reaching past it.
    assert_eq!(
        client.prune_earnings(&artist, &100, &100),
        2,
        "prune must stop at the retention boundary",
    );

    // The in-window record survives.
    assert!(client.try_get_earning(&artist, &2).is_ok());
    assert_eq!(client.get_earning_count(&artist), 3);
}

// ── Atomicity of metric updates and events (#44) ─────────────────────────

fn event_count(env: &Env) -> u32 {
    env.events().all().len()
}

#[test]
fn test_rejected_earning_leaves_no_aggregate_or_event() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin) = setup(&env);

    let artist = Address::generate(&env);
    let events_before = event_count(&env);

    // A non-positive payout is refused before anything is written or
    // announced.
    let result = client.try_record_earning(
        &artist,
        &make_bytes(&env, "c1"),
        &make_string(&env, "ui"),
        &Address::generate(&env),
        &0,
    );
    assert_eq!(result, Err(Ok(AnalyticsError::InvalidAmount)));

    assert_eq!(
        client.try_get_metrics(&artist),
        Err(Ok(AnalyticsError::NotFound)),
    );
    assert_eq!(client.get_earning_count(&artist), 0);
    assert_eq!(
        event_count(&env),
        events_before,
        "a rejected update must not emit a success event"
    );
}

#[test]
fn test_earning_overflow_keeps_prior_aggregate_and_events() {
// ── Overflow safety (#43) ────────────────────────────────────────────────

fn empty_metrics() -> ArtistMetrics {
    ArtistMetrics {
        total_earnings: 0,
        completed_count: 0,
        cancelled_count: 0,
        response_time_sum: 0,
        response_time_count: 0,
        satisfaction_score_sum: 0,
        satisfaction_score_count: 0,
        last_updated_ledger: 0,
    }
}

/// Write an aggregate straight to storage, so a counter can be positioned at
/// its maximum without recording billions of events to get there.
fn seed_metrics(
    env: &Env,
    client: &AnalyticsContractClient,
    artist: &Address,
    metrics: &ArtistMetrics,
) {
    env.as_contract(&client.address, || {
        env.storage()
            .persistent()
            .set(&DataKey::Metrics(artist.clone()), metrics);
    });
}

#[test]
fn test_record_earning_overflow_preserves_totals() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin) = setup(&env);

    let artist = Address::generate(&env);
    let client_addr = Address::generate(&env);
    let cat = make_string(&env, "ui");

    client.record_earning(
        &artist,
        &make_bytes(&env, "max"),
        &cat,
        &client_addr,
        &i128::MAX,
    );
    let metrics_before = client.get_metrics(&artist);
    let events_before = event_count(&env);

    let before = client.get_metrics(&artist);

    // One more unit would wrap the lifetime total, so it is refused with an
    // explicit error instead.
    let result = client.try_record_earning(
        &artist,
        &make_bytes(&env, "over"),
        &cat,
        &client_addr,
        &1,
    );
    assert_eq!(result, Err(Ok(AnalyticsError::ArithmeticOverflow)));

    // The single earlier payout is untouched and the failure adds no event.
    assert_eq!(client.get_metrics(&artist), metrics_before);
    assert_eq!(client.get_earning_count(&artist), 1);
    assert_eq!(event_count(&env), events_before);
}

#[test]
fn test_rejected_response_time_leaves_no_aggregate_or_event() {
    // The rejected update leaves both the aggregate and the earnings log
    // exactly as they were.
    assert_eq!(client.get_metrics(&artist), before);
    assert_eq!(client.get_earning_count(&artist), 1);
    assert_eq!(client.get_earning(&artist, &0).amount, i128::MAX);
}

#[test]
fn test_completed_count_overflow_leaves_no_earning_record() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin) = setup(&env);

    let artist = Address::generate(&env);
    let events_before = event_count(&env);

    let result = client.try_record_response_time(&artist, &0u64);
    assert_eq!(result, Err(Ok(AnalyticsError::InvalidAmount)));
    assert_eq!(
        client.try_get_metrics(&artist),
        Err(Ok(AnalyticsError::NotFound)),
    );
    assert_eq!(event_count(&env), events_before);
}

#[test]
fn test_response_time_overflow_keeps_prior_aggregate_and_events() {
    let mut metrics = empty_metrics();
    metrics.completed_count = u32::MAX;
    seed_metrics(&env, &client, &artist, &metrics);

    let result = client.try_record_earning(
        &artist,
        &make_bytes(&env, "over"),
        &make_string(&env, "ui"),
        &Address::generate(&env),
        &1_000,
    );
    assert_eq!(result, Err(Ok(AnalyticsError::ArithmeticOverflow)));

    assert_eq!(client.get_metrics(&artist), metrics);
    // The failure happened before the log append, so no index was consumed.
    assert_eq!(client.get_earning_count(&artist), 0);
    assert_eq!(
        client.try_get_earning(&artist, &0),
        Err(Ok(AnalyticsError::NotFound)),
    );
}

#[test]
fn test_response_time_sum_overflow_preserves_totals() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin) = setup(&env);

    let artist = Address::generate(&env);
    client.record_response_time(&artist, &u64::MAX);
    let metrics_before = client.get_metrics(&artist);
    let events_before = event_count(&env);

    let result = client.try_record_response_time(&artist, &1u64);
    assert_eq!(result, Err(Ok(AnalyticsError::ArithmeticOverflow)));
    assert_eq!(client.get_metrics(&artist), metrics_before);
    assert_eq!(event_count(&env), events_before);
}

#[test]
fn test_rejected_satisfaction_leaves_no_aggregate_or_event() {
    let before = client.get_metrics(&artist);

    let result = client.try_record_response_time(&artist, &1u64);
    assert_eq!(result, Err(Ok(AnalyticsError::ArithmeticOverflow)));

    assert_eq!(client.get_metrics(&artist), before);
    assert_eq!(client.get_avg_response_time(&artist), u64::MAX);
}

#[test]
fn test_satisfaction_sum_overflow_preserves_totals() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin) = setup(&env);

    let artist = Address::generate(&env);
    let events_before = event_count(&env);

    // Both ends of the invalid range: too low and too high.
    for bad_score in [5u32, 55u32] {
        let result = client.try_record_satisfaction(&artist, &bad_score);
        assert_eq!(result, Err(Ok(AnalyticsError::InvalidScore)));
    }

    assert_eq!(
        client.try_get_metrics(&artist),
        Err(Ok(AnalyticsError::NotFound)),
    );
    assert_eq!(event_count(&env), events_before);
}

#[test]
fn test_successful_update_emits_exactly_one_event() {
    let mut metrics = empty_metrics();
    metrics.satisfaction_score_sum = u32::MAX - 5;
    metrics.satisfaction_score_count = 1;
    seed_metrics(&env, &client, &artist, &metrics);

    let result = client.try_record_satisfaction(&artist, &10u32);
    assert_eq!(result, Err(Ok(AnalyticsError::ArithmeticOverflow)));
    assert_eq!(client.get_metrics(&artist), metrics);
}

#[test]
fn test_completion_rate_overflow_is_reported() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin) = setup(&env);

    let artist = Address::generate(&env);
    let mut metrics = empty_metrics();
    metrics.completed_count = u32::MAX;
    metrics.cancelled_count = 1;
    seed_metrics(&env, &client, &artist, &metrics);

    // The denominator would wrap, so the rate is reported as an error rather
    // than as a bogus percentage.
    assert_eq!(
        client.try_get_completion_rate(&artist),
        Err(Ok(AnalyticsError::ArithmeticOverflow)),
    );
}

#[test]
fn test_earning_index_overflow_leaves_no_partial_update() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin) = setup(&env);

    let artist = Address::generate(&env);
    let events_before = event_count(&env);

    client.record_earning(
        &artist,
        &make_bytes(&env, "c1"),
    env.as_contract(&client.address, || {
        env.storage()
            .persistent()
            .set(&DataKey::EarningCount(artist.clone()), &u32::MAX);
    });

    let result = client.try_record_earning(
        &artist,
        &make_bytes(&env, "over"),
        &make_string(&env, "ui"),
        &Address::generate(&env),
        &1_000,
    );

    // Validation, writes and event publication stay paired: one accepted
    // update, one announcement.
    assert_eq!(event_count(&env), events_before + 1);
    assert_eq!(client.get_metrics(&artist).total_earnings, 1_000);
    assert_eq!(result, Err(Ok(AnalyticsError::ArithmeticOverflow)));

    // Nothing was created for the artist and the index counter did not move.
    assert_eq!(
        client.try_get_metrics(&artist),
        Err(Ok(AnalyticsError::NotFound)),
    );
    assert_eq!(client.get_earning_count(&artist), u32::MAX);
}
