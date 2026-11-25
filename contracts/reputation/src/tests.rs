extern crate std;

use soroban_sdk::{
    testutils::{storage::Persistent as _, Address as _, Ledger as _},
    Address, Bytes, Env, String,
};

use crate::{
    ReputationContract, ReputationContractClient,
    errors::ReputationError,
    score::{MAX_RATING_X10, MIN_RATING_X10},
    types::{
        AppealStatus, DataKey, ReportReason, ReviewStatus, DEFAULT_APPEAL_WINDOW_LEDGERS,
        MAX_APPEAL_WINDOW_LEDGERS, MAX_QUEUE_SIZE,
    },
};

fn setup_with_id(env: &Env) -> (ReputationContractClient<'_>, Address, Address) {
    let contract_id = env.register_contract(None, ReputationContract);
    let client = ReputationContractClient::new(env, &contract_id);
    let admin = Address::generate(env);
    client.initialize(&admin);
    (client, admin, contract_id)
}

fn setup(env: &Env) -> (ReputationContractClient<'_>, Address) {
    let (client, admin, _contract_id) = setup_with_id(env);
    (client, admin)
}

fn ttl_of(env: &Env, contract_id: &Address, key: &DataKey) -> u32 {
    env.as_contract(contract_id, || env.storage().persistent().get_ttl(key))
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
    setup(&env);
}

#[test]
fn test_double_initialize_fails() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, admin) = setup(&env);
    let result = client.try_initialize(&admin);
    assert_eq!(result, Err(Ok(ReputationError::AlreadyInitialized)));
}

// ── Review submission ──────────────────────────────────────────────────────

#[test]
fn test_submit_review_succeeds() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin) = setup(&env);

    let artist = Address::generate(&env);
    let reviewer = Address::generate(&env);
    let review_id = make_bytes(&env, "r1");

    client.submit_review(
        &review_id,
        &artist,
        &reviewer,
        &45u32,
        &make_string(&env, "Great work!"),
    );

    let r = client.get_review(&review_id);
    assert_eq!(r.status, ReviewStatus::Active);
    assert_eq!(r.rating_x10, 45);
    assert_eq!(r.artist, artist);
}

#[test]
fn test_submit_review_invalid_rating_too_low_fails() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin) = setup(&env);
    let artist = Address::generate(&env);
    let reviewer = Address::generate(&env);
    let result = client.try_submit_review(
        &make_bytes(&env, "r2"),
        &artist,
        &reviewer,
        &5u32,
        &make_string(&env, "bad"),
    );
    assert_eq!(result, Err(Ok(ReputationError::InvalidRating)));
}

#[test]
fn test_submit_review_invalid_rating_too_high_fails() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin) = setup(&env);
    let artist = Address::generate(&env);
    let reviewer = Address::generate(&env);
    let result = client.try_submit_review(
        &make_bytes(&env, "r3"),
        &artist,
        &reviewer,
        &55u32,
        &make_string(&env, "bad"),
    );
    assert_eq!(result, Err(Ok(ReputationError::InvalidRating)));
}

#[test]
fn test_submit_review_boundary_ratings_ok() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin) = setup(&env);
    let artist = Address::generate(&env);
    // Min
    client.submit_review(
        &make_bytes(&env, "r4"),
        &artist,
        &Address::generate(&env),
        &10u32,
        &make_string(&env, "min"),
    );
    // Max
    client.submit_review(
        &make_bytes(&env, "r5"),
        &artist,
        &Address::generate(&env),
        &50u32,
        &make_string(&env, "max"),
    );
}

// ── Duplicate reviews (#48) ────────────────────────────────────────────────

/// A client may leave only one review per artist, whichever review id they use,
/// and the rejected attempt leaves the aggregate untouched.
#[test]
fn test_duplicate_review_from_same_client_keeps_aggregate() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin) = setup(&env);

    let artist = Address::generate(&env);
    let reviewer = Address::generate(&env);
    client.submit_review(
        &make_bytes(&env, "d1"),
        &artist,
        &reviewer,
        &40u32,
        &make_string(&env, "good"),
    );
    let before = client.get_aggregate(&artist);

    // Same client, same artist — a different review id must not be enough.
    let result = client.try_submit_review(
        &make_bytes(&env, "d2"),
        &artist,
        &reviewer,
        &50u32,
        &make_string(&env, "again"),
    );
    assert_eq!(result, Err(Ok(ReputationError::DuplicateReview)));
    assert_eq!(client.get_aggregate(&artist), before);
    assert_eq!(client.get_score(&artist).total_reviews, 1);
    assert_eq!(client.get_score(&artist).rating_sum, 40);
}

/// Reusing a review id is rejected as a duplicate too, even from a new client.
#[test]
fn test_duplicate_review_id_is_rejected() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin) = setup(&env);

    let artist = Address::generate(&env);
    client.submit_review(
        &make_bytes(&env, "same"),
        &artist,
        &Address::generate(&env),
        &30u32,
        &make_string(&env, "first"),
    );
    let before = client.get_aggregate(&artist);

    let result = client.try_submit_review(
        &make_bytes(&env, "same"),
        &artist,
        &Address::generate(&env),
        &50u32,
        &make_string(&env, "overwrite"),
    );
    assert_eq!(result, Err(Ok(ReputationError::DuplicateReview)));
    assert_eq!(client.get_aggregate(&artist), before);
    // The original record is intact.
    assert_eq!(client.get_review(&make_bytes(&env, "same")).rating_x10, 30);
}

/// The unique key is per artist: the same client may still review other artists.
#[test]
fn test_same_client_can_review_different_artists() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin) = setup(&env);

    let reviewer = Address::generate(&env);
    let artist_a = Address::generate(&env);
    let artist_b = Address::generate(&env);

    client.submit_review(
        &make_bytes(&env, "a1"),
        &artist_a,
        &reviewer,
        &40u32,
        &make_string(&env, "a"),
    );
    client.submit_review(
        &make_bytes(&env, "b1"),
        &artist_b,
        &reviewer,
        &20u32,
        &make_string(&env, "b"),
    );

    assert_eq!(client.get_score(&artist_a).total_reviews, 1);
    assert_eq!(client.get_score(&artist_b).total_reviews, 1);
}

/// Removing a review does not free the key: the same client cannot file a
/// replacement rating after moderation.
#[test]
fn test_removed_review_still_blocks_a_new_one() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin) = setup(&env);

    let artist = Address::generate(&env);
    let reviewer = Address::generate(&env);
    let review_id = make_bytes(&env, "gone");
    client.submit_review(&review_id, &artist, &reviewer, &10u32, &make_string(&env, "bad"));
    client.report_review(&review_id, &Address::generate(&env), &ReportReason::Spam, &make_string(&env, "s"));
    client.moderate_review(&review_id, &ReviewStatus::Removed, &make_string(&env, "removed"));

    let result = client.try_submit_review(
        &make_bytes(&env, "retry"),
        &artist,
        &reviewer,
        &50u32,
        &make_string(&env, "retry"),
    );
    assert_eq!(result, Err(Ok(ReputationError::DuplicateReview)));
}

// ── Review reporting ───────────────────────────────────────────────────────

#[test]
fn test_report_review_transitions_to_under_review() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin) = setup(&env);

    let artist = Address::generate(&env);
    let reviewer = Address::generate(&env);
    let reporter = Address::generate(&env);
    let review_id = make_bytes(&env, "rep1");

    client.submit_review(
        &review_id,
        &artist,
        &reviewer,
        &30u32,
        &make_string(&env, "ok"),
    );

    client.report_review(
        &review_id,
        &reporter,
        &ReportReason::Spam,
        &make_string(&env, "this is spam"),
    );

    let r = client.get_review(&review_id);
    assert_eq!(r.status, ReviewStatus::UnderReview);
}

#[test]
fn test_report_review_increments_count() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin) = setup(&env);

    let artist = Address::generate(&env);
    let reviewer = Address::generate(&env);
    let reporter1 = Address::generate(&env);
    let reporter2 = Address::generate(&env);
    let review_id = make_bytes(&env, "rep2");

    client.submit_review(&review_id, &artist, &reviewer, &40u32, &make_string(&env, "ok"));
    client.report_review(&review_id, &reporter1, &ReportReason::Abuse, &make_string(&env, "a"));
    // After first report review is UnderReview; second reporter can still add
    // a report from a different address but status stays UnderReview
    // (note: re-report by same reporter should fail)
    let result = client.try_report_review(
        &review_id, &reporter1, &ReportReason::Abuse, &make_string(&env, "dup"),
    );
    assert_eq!(result, Err(Ok(ReputationError::AlreadyReported)));

    // Different reporter also cannot report a UnderReview - only Active/Cleared
    let result2 = client.try_report_review(
        &review_id, &reporter2, &ReportReason::Misleading, &make_string(&env, "b"),
    );
    assert_eq!(result2, Err(Ok(ReputationError::InvalidStatus)));
}

#[test]
fn test_duplicate_reporter_is_rejected_without_adding_a_report() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin) = setup(&env);

    let artist = Address::generate(&env);
    let reviewer = Address::generate(&env);
    let reporter = Address::generate(&env);
    let review_id = make_bytes(&env, "dup-reporter");

    client.submit_review(&review_id, &artist, &reviewer, &30u32, &make_string(&env, "ok"));
    client.report_review(&review_id, &reporter, &ReportReason::Spam, &make_string(&env, "first"));

    // The first report moved the review to UnderReview; a second report from
    // the same address is rejected as a duplicate before the status check, and
    // must not consume a report slot.
    let result = client.try_report_review(
        &review_id,
        &reporter,
        &ReportReason::Abuse,
        &make_string(&env, "second"),
    );
    assert_eq!(result, Err(Ok(ReputationError::AlreadyReported)));
    assert_eq!(client.get_report_count(&review_id), 1);
}

#[test]
fn test_new_report_after_prior_report_is_dismissed_is_allowed() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin) = setup(&env);

    let artist = Address::generate(&env);
    let reviewer = Address::generate(&env);
    let reporter1 = Address::generate(&env);
    let reporter2 = Address::generate(&env);
    let review_id = make_bytes(&env, "dismissed");

    client.submit_review(&review_id, &artist, &reviewer, &40u32, &make_string(&env, "ok"));
    client.report_review(&review_id, &reporter1, &ReportReason::Spam, &make_string(&env, "spam"));
    client.moderate_review(&review_id, &ReviewStatus::Cleared, &make_string(&env, "no violation"));

    // A cleared review is reportable again, and the dismissed report still
    // counts toward the per-review budget.
    client.report_review(&review_id, &reporter2, &ReportReason::Misleading, &make_string(&env, "still bad"));
    assert_eq!(client.get_report_count(&review_id), 2);
    assert_eq!(client.get_review(&review_id).status, ReviewStatus::UnderReview);
}

#[test]
fn test_same_reporter_cannot_report_again_after_dismissal() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin) = setup(&env);

    let artist = Address::generate(&env);
    let reviewer = Address::generate(&env);
    let reporter = Address::generate(&env);
    let review_id = make_bytes(&env, "repeat-after-clear");

    client.submit_review(&review_id, &artist, &reviewer, &20u32, &make_string(&env, "meh"));
    client.report_review(&review_id, &reporter, &ReportReason::Spam, &make_string(&env, "spam"));
    client.moderate_review(&review_id, &ReviewStatus::Cleared, &make_string(&env, "dismissed"));

    let result = client.try_report_review(
        &review_id,
        &reporter,
        &ReportReason::Abuse,
        &make_string(&env, "again"),
    );
    assert_eq!(result, Err(Ok(ReputationError::AlreadyReported)));
    assert_eq!(client.get_report_count(&review_id), 1);
}

#[test]
fn test_report_removed_review_fails() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin) = setup(&env);

    let artist = Address::generate(&env);
    let reviewer = Address::generate(&env);
    let reporter = Address::generate(&env);
    let review_id = make_bytes(&env, "rep3");

    client.submit_review(&review_id, &artist, &reviewer, &20u32, &make_string(&env, "meh"));
    client.report_review(&review_id, &reporter, &ReportReason::Spam, &make_string(&env, "s"));
    client.moderate_review(&review_id, &ReviewStatus::Removed, &make_string(&env, "spam"));

    let reporter2 = Address::generate(&env);
    let result = client.try_report_review(
        &review_id, &reporter2, &ReportReason::Other, &make_string(&env, "x"),
    );
    assert_eq!(result, Err(Ok(ReputationError::InvalidStatus)));
}

// ── Admin moderation ───────────────────────────────────────────────────────

#[test]
fn test_moderate_review_remove() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin) = setup(&env);

    let artist = Address::generate(&env);
    let reviewer = Address::generate(&env);
    let reporter = Address::generate(&env);
    let review_id = make_bytes(&env, "mod1");

    client.submit_review(&review_id, &artist, &reviewer, &30u32, &make_string(&env, "ok"));
    client.report_review(&review_id, &reporter, &ReportReason::Spam, &make_string(&env, "s"));
    client.moderate_review(&review_id, &ReviewStatus::Removed, &make_string(&env, "spam confirmed"));

    assert_eq!(client.get_review(&review_id).status, ReviewStatus::Removed);
    assert_eq!(client.get_moderation_count(&review_id), 1);
    let entry = client.get_moderation_entry(&review_id, &0);
    assert_eq!(entry.new_status, ReviewStatus::Removed);
}

#[test]
fn test_moderate_review_clear() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin) = setup(&env);

    let artist = Address::generate(&env);
    let reviewer = Address::generate(&env);
    let reporter = Address::generate(&env);
    let review_id = make_bytes(&env, "mod2");

    client.submit_review(&review_id, &artist, &reviewer, &45u32, &make_string(&env, "fine"));
    client.report_review(&review_id, &reporter, &ReportReason::Other, &make_string(&env, "o"));
    client.moderate_review(&review_id, &ReviewStatus::Cleared, &make_string(&env, "no violation"));

    assert_eq!(client.get_review(&review_id).status, ReviewStatus::Cleared);
}

#[test]
fn test_moderate_non_under_review_fails() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin) = setup(&env);

    let artist = Address::generate(&env);
    let reviewer = Address::generate(&env);
    let review_id = make_bytes(&env, "mod3");

    client.submit_review(&review_id, &artist, &reviewer, &40u32, &make_string(&env, "nice"));

    // Status is Active, not UnderReview
    let result = client.try_moderate_review(
        &review_id, &ReviewStatus::Removed, &make_string(&env, "no"),
    );
    assert_eq!(result, Err(Ok(ReputationError::InvalidStatus)));
}

#[test]
fn test_moderation_queue_size_increments() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin) = setup(&env);

    let artist = Address::generate(&env);
    let reviewer = Address::generate(&env);
    let reporter = Address::generate(&env);

    let r1 = make_bytes(&env, "q1");
    let r2 = make_bytes(&env, "q2");

    // Distinct reviewers: `(artist, reviewer)` is the unique review key (#48).
    client.submit_review(&r1, &artist, &reviewer, &30u32, &make_string(&env, "a"));
    client.submit_review(&r2, &artist, &Address::generate(&env), &40u32, &make_string(&env, "b"));

    client.report_review(&r1, &reporter, &ReportReason::Spam, &make_string(&env, "x"));
    let reporter2 = Address::generate(&env);
    client.report_review(&r2, &reporter2, &ReportReason::Abuse, &make_string(&env, "y"));

    assert_eq!(client.get_moderation_queue_size(), 2);
}

// ── Appeal mechanism ───────────────────────────────────────────────────────

#[test]
fn test_file_and_resolve_appeal_upheld() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin) = setup(&env);

    let artist = Address::generate(&env);
    let reviewer = Address::generate(&env);
    let reporter = Address::generate(&env);
    let review_id = make_bytes(&env, "ap1");

    client.submit_review(&review_id, &artist, &reviewer, &35u32, &make_string(&env, "good"));
    client.report_review(&review_id, &reporter, &ReportReason::Spam, &make_string(&env, "s"));
    client.moderate_review(&review_id, &ReviewStatus::Removed, &make_string(&env, "removed"));

    client.file_appeal(&review_id, &artist, &make_string(&env, "I disagree"));

    let appeal = client.get_appeal(&review_id);
    assert_eq!(appeal.status, AppealStatus::Pending);

    client.resolve_appeal(&review_id, &AppealStatus::Upheld);

    let appeal = client.get_appeal(&review_id);
    assert_eq!(appeal.status, AppealStatus::Upheld);

    // Review should be reinstated
    assert_eq!(client.get_review(&review_id).status, ReviewStatus::Active);
}

#[test]
fn test_file_and_resolve_appeal_denied() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin) = setup(&env);

    let artist = Address::generate(&env);
    let reviewer = Address::generate(&env);
    let reporter = Address::generate(&env);
    let review_id = make_bytes(&env, "ap2");

    client.submit_review(&review_id, &artist, &reviewer, &20u32, &make_string(&env, "meh"));
    client.report_review(&review_id, &reporter, &ReportReason::Abuse, &make_string(&env, "a"));
    client.moderate_review(&review_id, &ReviewStatus::Removed, &make_string(&env, "removed"));

    client.file_appeal(&review_id, &reviewer, &make_string(&env, "Not abuse"));

    client.resolve_appeal(&review_id, &AppealStatus::Denied);

    let appeal = client.get_appeal(&review_id);
    assert_eq!(appeal.status, AppealStatus::Denied);
    // Review stays Removed
    assert_eq!(client.get_review(&review_id).status, ReviewStatus::Removed);
}

#[test]
fn test_file_appeal_on_active_review_fails() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin) = setup(&env);

    let artist = Address::generate(&env);
    let reviewer = Address::generate(&env);
    let review_id = make_bytes(&env, "ap3");

    client.submit_review(&review_id, &artist, &reviewer, &45u32, &make_string(&env, "ok"));

    let result = client.try_file_appeal(&review_id, &artist, &make_string(&env, "why?"));
    assert_eq!(result, Err(Ok(ReputationError::InvalidStatus)));
}

#[test]
fn test_duplicate_appeal_fails() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin) = setup(&env);

    let artist = Address::generate(&env);
    let reviewer = Address::generate(&env);
    let reporter = Address::generate(&env);
    let review_id = make_bytes(&env, "ap4");

    client.submit_review(&review_id, &artist, &reviewer, &30u32, &make_string(&env, "ok"));
    client.report_review(&review_id, &reporter, &ReportReason::Spam, &make_string(&env, "s"));
    client.moderate_review(&review_id, &ReviewStatus::Removed, &make_string(&env, "removed"));

    client.file_appeal(&review_id, &artist, &make_string(&env, "reason 1"));
    let result = client.try_file_appeal(&review_id, &artist, &make_string(&env, "reason 2"));
    assert_eq!(result, Err(Ok(ReputationError::AppealAlreadyExists)));
}

#[test]
fn test_escalate_appeal() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin) = setup(&env);

    let artist = Address::generate(&env);
    let reviewer = Address::generate(&env);
    let reporter = Address::generate(&env);
    let review_id = make_bytes(&env, "ap5");

    client.submit_review(&review_id, &artist, &reviewer, &20u32, &make_string(&env, "meh"));
    client.report_review(&review_id, &reporter, &ReportReason::Abuse, &make_string(&env, "a"));
    client.moderate_review(&review_id, &ReviewStatus::Removed, &make_string(&env, "removed"));

    client.file_appeal(&review_id, &artist, &make_string(&env, "escalate please"));
    client.escalate_appeal(&review_id, &artist);

    assert_eq!(client.get_appeal(&review_id).status, AppealStatus::Escalated);
}

/// A review that moderation has `Removed`, with a `Pending` appeal against it.
fn removed_review_with_appeal(
    env: &Env,
    client: &ReputationContractClient<'_>,
    suffix: &str,
    rating_x10: u32,
) -> (Address, Bytes) {
    let artist = Address::generate(env);
    let reviewer = Address::generate(env);
    let reporter = Address::generate(env);
    let review_id = make_bytes(env, suffix);

    client.submit_review(
        &review_id,
        &artist,
        &reviewer,
        &rating_x10,
        &make_string(env, "review"),
    );
    client.report_review(
        &review_id,
        &reporter,
        &ReportReason::Spam,
        &make_string(env, "report"),
    );
    client.moderate_review(&review_id, &ReviewStatus::Removed, &make_string(env, "removed"));
    client.file_appeal(&review_id, &artist, &make_string(env, "appeal"));

    (artist, review_id)
}

#[test]
fn test_appeal_status_terminal_model() {
    assert!(!AppealStatus::Pending.is_terminal());
    assert!(!AppealStatus::Pending.is_decided());

    assert!(AppealStatus::Upheld.is_terminal());
    assert!(AppealStatus::Upheld.is_decided());

    assert!(AppealStatus::Denied.is_terminal());
    assert!(AppealStatus::Denied.is_decided());

    // Escalated is terminal for this contract but carries no decision: the
    // appeal belongs to the dispute arbiter from that point on.
    assert!(AppealStatus::Escalated.is_terminal());
    assert!(!AppealStatus::Escalated.is_decided());
}

#[test]
fn test_repeated_upheld_decision_is_rejected() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin) = setup(&env);

    let (artist, review_id) = removed_review_with_appeal(&env, &client, "dup-upheld", 40);
    client.resolve_appeal(&review_id, &AppealStatus::Upheld);
    let score_after_decision = client.get_score(&artist);

    let result = client.try_resolve_appeal(&review_id, &AppealStatus::Upheld);
    assert_eq!(result, Err(Ok(ReputationError::InvalidStatus)));

    // The first decision stands, and the review is not reinstated twice.
    assert_eq!(client.get_appeal(&review_id).status, AppealStatus::Upheld);
    assert_eq!(client.get_review(&review_id).status, ReviewStatus::Active);
    assert_eq!(client.get_score(&artist), score_after_decision);
}

#[test]
fn test_conflicting_decision_after_denial_is_rejected() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin) = setup(&env);

    let (artist, review_id) = removed_review_with_appeal(&env, &client, "conflict", 40);
    client.resolve_appeal(&review_id, &AppealStatus::Denied);
    let score_after_decision = client.get_score(&artist);
    assert_eq!(score_after_decision.active_reviews, 0);
    assert_eq!(score_after_decision.removed_reviews, 1);

    // A later `Upheld` must not overturn the denial.
    let result = client.try_resolve_appeal(&review_id, &AppealStatus::Upheld);
    assert_eq!(result, Err(Ok(ReputationError::InvalidStatus)));

    assert_eq!(client.get_appeal(&review_id).status, AppealStatus::Denied);
    assert_eq!(client.get_review(&review_id).status, ReviewStatus::Removed);
    assert_eq!(client.get_score(&artist), score_after_decision);
}

#[test]
fn test_no_decision_is_allowed_after_escalation() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin) = setup(&env);

    let (artist, review_id) = removed_review_with_appeal(&env, &client, "escalated", 30);
    client.escalate_appeal(&review_id, &artist);
    let score_after_escalation = client.get_score(&artist);

    for decision in [AppealStatus::Upheld, AppealStatus::Denied] {
        let result = client.try_resolve_appeal(&review_id, &decision);
        assert_eq!(result, Err(Ok(ReputationError::InvalidStatus)));
    }

    assert_eq!(client.get_appeal(&review_id).status, AppealStatus::Escalated);
    assert_eq!(client.get_review(&review_id).status, ReviewStatus::Removed);
    assert_eq!(client.get_score(&artist), score_after_escalation);
}

#[test]
fn test_third_party_cannot_file_appeal() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin) = setup(&env);

    let artist = Address::generate(&env);
    let reviewer = Address::generate(&env);
    let third_party = Address::generate(&env);
    let reporter = Address::generate(&env);
    let review_id = make_bytes(&env, "ap6");

    client.submit_review(&review_id, &artist, &reviewer, &30u32, &make_string(&env, "ok"));
    client.report_review(&review_id, &reporter, &ReportReason::Spam, &make_string(&env, "s"));
    client.moderate_review(&review_id, &ReviewStatus::Removed, &make_string(&env, "removed"));

    let result = client.try_file_appeal(&review_id, &third_party, &make_string(&env, "i want to appeal"));
    assert_eq!(result, Err(Ok(ReputationError::Unauthorized)));
}

// ── Reputation aggregate (#974) ─────────────────────────────────────────────

#[test]
fn test_score_reflects_submitted_reviews() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin) = setup(&env);

    let artist = Address::generate(&env);
    client.submit_review(
        &make_bytes(&env, "s1"),
        &artist,
        &Address::generate(&env),
        &50u32,
        &make_string(&env, "great"),
    );
    client.submit_review(
        &make_bytes(&env, "s2"),
        &artist,
        &Address::generate(&env),
        &30u32,
        &make_string(&env, "ok"),
    );

    let score = client.get_score(&artist);
    assert_eq!(score.total_reviews, 2);
    assert_eq!(score.active_reviews, 2);
    assert_eq!(score.rating_sum, 80);
    assert_eq!(score.average_rating_x10, 40);
    assert_eq!(score.removed_reviews, 0);
    assert_eq!(score.appeal_count, 0);
}

#[test]
fn test_score_at_the_lowest_accepted_rating() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin) = setup(&env);

    let artist = Address::generate(&env);
    client.submit_review(
        &make_bytes(&env, "low"),
        &artist,
        &Address::generate(&env),
        &MIN_RATING_X10,
        &make_string(&env, "lowest accepted"),
    );

    let score = client.get_score(&artist);
    assert_eq!(score.total_reviews, 1);
    assert_eq!(score.active_reviews, 1);
    assert_eq!(score.rating_sum, MIN_RATING_X10);
    assert_eq!(score.average_rating_x10, MIN_RATING_X10);
    // (10 + 30*3) / (1 + 3) = 100 / 4 = 25.
    assert_eq!(score.weighted_score_x10, 25);
}

#[test]
fn test_score_at_the_highest_accepted_rating() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin) = setup(&env);

    let artist = Address::generate(&env);
    client.submit_review(
        &make_bytes(&env, "high"),
        &artist,
        &Address::generate(&env),
        &MAX_RATING_X10,
        &make_string(&env, "highest accepted"),
    );

    let score = client.get_score(&artist);
    assert_eq!(score.total_reviews, 1);
    assert_eq!(score.active_reviews, 1);
    assert_eq!(score.rating_sum, MAX_RATING_X10);
    assert_eq!(score.average_rating_x10, MAX_RATING_X10);
    // (50 + 30*3) / (1 + 3) = 140 / 4 = 35.
    assert_eq!(score.weighted_score_x10, 35);
}

#[test]
fn test_score_mixes_the_lowest_and_highest_ratings() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin) = setup(&env);

    let artist = Address::generate(&env);
    client.submit_review(
        &make_bytes(&env, "mix-low"),
        &artist,
        &Address::generate(&env),
        &MIN_RATING_X10,
        &make_string(&env, "low"),
    );
    client.submit_review(
        &make_bytes(&env, "mix-high"),
        &artist,
        &Address::generate(&env),
        &MAX_RATING_X10,
        &make_string(&env, "high"),
    );

    let score = client.get_score(&artist);
    assert_eq!(score.total_reviews, 2);
    assert_eq!(score.active_reviews, 2);
    assert_eq!(score.rating_sum, MIN_RATING_X10 + MAX_RATING_X10);
    assert_eq!(score.average_rating_x10, 30);
    // (60 + 90) / (2 + 3) = 150 / 5 = 30.
    assert_eq!(score.weighted_score_x10, 30);
}

#[test]
fn test_score_stays_consistent_after_removing_a_review() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin) = setup(&env);

    let artist = Address::generate(&env);
    let low_id = make_bytes(&env, "rm-low");
    let high_id = make_bytes(&env, "rm-high");
    client.submit_review(&low_id, &artist, &Address::generate(&env), &MIN_RATING_X10, &make_string(&env, "low"));
    client.submit_review(&high_id, &artist, &Address::generate(&env), &MAX_RATING_X10, &make_string(&env, "high"));

    // Remove only the highest-rated review.
    client.report_review(&high_id, &Address::generate(&env), &ReportReason::Spam, &make_string(&env, "s"));
    client.moderate_review(&high_id, &ReviewStatus::Removed, &make_string(&env, "removed"));

    let score = client.get_score(&artist);
    assert_eq!(score.total_reviews, 2);
    assert_eq!(score.active_reviews, 1);
    assert_eq!(score.removed_reviews, 1);
    assert_eq!(score.rating_sum, MIN_RATING_X10);
    assert_eq!(score.average_rating_x10, MIN_RATING_X10);
    // Back to a single lowest-rated review: (10 + 90) / 4 = 25.
    assert_eq!(score.weighted_score_x10, 25);
}

#[test]
fn test_score_of_unknown_subject_is_neutral() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin) = setup(&env);

    let score = client.get_score(&Address::generate(&env));
    assert_eq!(score.total_reviews, 0);
    assert_eq!(score.average_rating_x10, 0);
    // The weighted score floors at the neutral prior (3.0), never below.
    assert_eq!(score.weighted_score_x10, 30);
}

#[test]
fn test_moderation_removes_a_review_from_the_score() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin) = setup(&env);

    let artist = Address::generate(&env);
    let review_id = make_bytes(&env, "m1");
    client.submit_review(&review_id, &artist, &Address::generate(&env), &50u32, &make_string(&env, "great"));
    client.report_review(&review_id, &Address::generate(&env), &ReportReason::Spam, &make_string(&env, "s"));
    client.moderate_review(&review_id, &ReviewStatus::Removed, &make_string(&env, "removed"));

    let score = client.get_score(&artist);
    assert_eq!(score.active_reviews, 0);
    assert_eq!(score.removed_reviews, 1);
    assert_eq!(score.average_rating_x10, 0);
    assert_eq!(score.moderation_count, 1);
}

#[test]
fn test_upheld_appeal_restores_the_score() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin) = setup(&env);

    let artist = Address::generate(&env);
    let review_id = make_bytes(&env, "m2");
    client.submit_review(&review_id, &artist, &Address::generate(&env), &40u32, &make_string(&env, "good"));
    client.report_review(&review_id, &Address::generate(&env), &ReportReason::Abuse, &make_string(&env, "a"));
    client.moderate_review(&review_id, &ReviewStatus::Removed, &make_string(&env, "removed"));
    assert_eq!(client.get_score(&artist).active_reviews, 0);

    client.file_appeal(&review_id, &artist, &make_string(&env, "unfair"));
    client.resolve_appeal(&review_id, &AppealStatus::Upheld);

    let score = client.get_score(&artist);
    assert_eq!(score.active_reviews, 1);
    assert_eq!(score.rating_sum, 40);
    assert_eq!(score.average_rating_x10, 40);
    assert_eq!(score.removed_reviews, 0);
    assert_eq!(score.appeal_count, 1);
}

// ── Aggregate correctness after moderation (#49) ────────────────────────────

/// Removing a review takes exactly that rating out of the aggregate, and a
/// repeated moderation call cannot take it out twice.
#[test]
fn test_moderation_adjusts_aggregate_exactly_once() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin) = setup(&env);

    let artist = Address::generate(&env);
    let removed_id = make_bytes(&env, "agg1");
    client.submit_review(&removed_id, &artist, &Address::generate(&env), &40u32, &make_string(&env, "good"));
    client.submit_review(&make_bytes(&env, "agg2"), &artist, &Address::generate(&env), &20u32, &make_string(&env, "meh"));

    let before = client.get_aggregate(&artist);
    assert_eq!(before.total_reviews, 2);
    assert_eq!(before.active_reviews, 2);
    assert_eq!(before.rating_sum, 60);

    client.report_review(&removed_id, &Address::generate(&env), &ReportReason::Abuse, &make_string(&env, "a"));
    client.moderate_review(&removed_id, &ReviewStatus::Removed, &make_string(&env, "removed"));

    let after = client.get_aggregate(&artist);
    assert_eq!(after.total_reviews, 2);
    assert_eq!(after.active_reviews, 1);
    assert_eq!(after.rating_sum, 20, "only the removed rating leaves the sum");
    assert_eq!(after.removed_reviews, 1);
    assert_eq!(after.moderation_count, 1);

    // The review is no longer `UnderReview`, so moderation cannot run again.
    let retry = client.try_moderate_review(&removed_id, &ReviewStatus::Removed, &make_string(&env, "again"));
    assert_eq!(retry, Err(Ok(ReputationError::InvalidStatus)));
    assert_eq!(client.get_aggregate(&artist), after);
}

/// Clearing a reported review leaves its rating in the aggregate; only the
/// moderation counter moves.
#[test]
fn test_clearing_a_review_keeps_its_rating_in_the_aggregate() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin) = setup(&env);

    let artist = Address::generate(&env);
    let review_id = make_bytes(&env, "agg3");
    client.submit_review(&review_id, &artist, &Address::generate(&env), &45u32, &make_string(&env, "fine"));
    client.report_review(&review_id, &Address::generate(&env), &ReportReason::Other, &make_string(&env, "o"));
    client.moderate_review(&review_id, &ReviewStatus::Cleared, &make_string(&env, "no violation"));

    let score = client.get_score(&artist);
    assert_eq!(score.active_reviews, 1);
    assert_eq!(score.rating_sum, 45);
    assert_eq!(score.average_rating_x10, 45);
    assert_eq!(score.removed_reviews, 0);
    assert_eq!(score.moderation_count, 1);
}

/// An upheld appeal restores the rating exactly once, and resolving the same
/// appeal again changes nothing.
#[test]
fn test_restoration_adjusts_aggregate_exactly_once() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin) = setup(&env);

    let artist = Address::generate(&env);
    let review_id = make_bytes(&env, "agg4");
    client.submit_review(&review_id, &artist, &Address::generate(&env), &30u32, &make_string(&env, "ok"));
    client.report_review(&review_id, &Address::generate(&env), &ReportReason::Misleading, &make_string(&env, "m"));
    client.moderate_review(&review_id, &ReviewStatus::Removed, &make_string(&env, "removed"));

    let removed = client.get_aggregate(&artist);
    assert_eq!(removed.active_reviews, 0);
    assert_eq!(removed.rating_sum, 0);
    assert_eq!(removed.removed_reviews, 1);

    client.file_appeal(&review_id, &artist, &make_string(&env, "unfair"));
    client.resolve_appeal(&review_id, &AppealStatus::Upheld);

    let restored = client.get_aggregate(&artist);
    assert_eq!(restored.active_reviews, 1);
    assert_eq!(restored.rating_sum, 30);
    assert_eq!(restored.removed_reviews, 0);
    assert_eq!(restored.appeal_count, 1);
    assert_eq!(restored.moderation_count, 1);

    // The appeal is already resolved, so it cannot be upheld a second time.
    let retry = client.try_resolve_appeal(&review_id, &AppealStatus::Upheld);
    assert_eq!(retry, Err(Ok(ReputationError::InvalidStatus)));
    assert_eq!(client.get_aggregate(&artist), restored);
}

// ── Appeal window (#976) ────────────────────────────────────────────────────

#[test]
fn test_appeal_window_default_is_reported() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin) = setup(&env);
    assert_eq!(client.get_appeal_window(), DEFAULT_APPEAL_WINDOW_LEDGERS);
}

#[test]
fn test_appeal_deadline_is_review_ledger_plus_window() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin) = setup(&env);

    let artist = Address::generate(&env);
    let review_id = make_bytes(&env, "w1");
    client.submit_review(&review_id, &artist, &Address::generate(&env), &30u32, &make_string(&env, "ok"));

    assert_eq!(client.get_appeal_deadline(&review_id), DEFAULT_APPEAL_WINDOW_LEDGERS);
    assert_eq!(client.ledgers_to_appeal(&review_id), DEFAULT_APPEAL_WINDOW_LEDGERS);
}

#[test]
fn test_set_appeal_window_rejects_out_of_range() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, admin) = setup(&env);

    for bad in [0u32, MAX_APPEAL_WINDOW_LEDGERS + 1] {
        let result = client.try_set_appeal_window(&admin, &bad);
        assert_eq!(result, Err(Ok(ReputationError::InvalidAppealWindow)));
    }
    client.set_appeal_window(&admin, &100u32);
    assert_eq!(client.get_appeal_window(), 100);
}

#[test]
fn test_file_appeal_immediately_before_the_boundary_is_allowed() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, admin) = setup(&env);
    // A 10-ledger window keeps the ledger jump small; jumping to the default
    // ~17k-ledger window would archive the contract's instance entry in the
    // test host.
    client.set_appeal_window(&admin, &10u32);

    let artist = Address::generate(&env);
    let review_id = make_bytes(&env, "w-before");
    client.submit_review(&review_id, &artist, &Address::generate(&env), &30u32, &make_string(&env, "ok"));
    client.report_review(&review_id, &Address::generate(&env), &ReportReason::Spam, &make_string(&env, "s"));
    client.moderate_review(&review_id, &ReviewStatus::Removed, &make_string(&env, "removed"));

    env.ledger().with_mut(|l| l.sequence_number = 9);
    assert_eq!(client.get_appeal_deadline(&review_id), 10);
    assert_eq!(client.ledgers_to_appeal(&review_id), 1);
    client.file_appeal(&review_id, &artist, &make_string(&env, "one ledger left"));
    assert_eq!(client.get_appeal(&review_id).status, AppealStatus::Pending);
}

#[test]
fn test_file_appeal_at_the_boundary_is_allowed() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, admin) = setup(&env);
    client.set_appeal_window(&admin, &10u32);

    let artist = Address::generate(&env);
    let review_id = make_bytes(&env, "w2");
    client.submit_review(&review_id, &artist, &Address::generate(&env), &30u32, &make_string(&env, "ok"));
    client.report_review(&review_id, &Address::generate(&env), &ReportReason::Spam, &make_string(&env, "s"));
    client.moderate_review(&review_id, &ReviewStatus::Removed, &make_string(&env, "removed"));

    env.ledger().with_mut(|l| l.sequence_number = 10);
    assert_eq!(client.get_appeal_deadline(&review_id), 10);
    // Zero ledgers remain, but the deadline ledger itself is inclusive.
    assert_eq!(client.ledgers_to_appeal(&review_id), 0);
    client.file_appeal(&review_id, &artist, &make_string(&env, "just in time"));
    assert_eq!(client.get_appeal(&review_id).status, AppealStatus::Pending);
}

#[test]
fn test_file_appeal_past_the_window_is_rejected() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, admin) = setup(&env);
    client.set_appeal_window(&admin, &10u32);

    let artist = Address::generate(&env);
    let review_id = make_bytes(&env, "w3");
    client.submit_review(&review_id, &artist, &Address::generate(&env), &30u32, &make_string(&env, "ok"));
    client.report_review(&review_id, &Address::generate(&env), &ReportReason::Spam, &make_string(&env, "s"));
    client.moderate_review(&review_id, &ReviewStatus::Removed, &make_string(&env, "removed"));

    env.ledger().with_mut(|l| l.sequence_number = 11);
    assert_eq!(client.get_appeal_deadline(&review_id), 10);
    assert_eq!(client.ledgers_to_appeal(&review_id), 0);
    let result = client.try_file_appeal(&review_id, &artist, &make_string(&env, "too late"));
    assert_eq!(result, Err(Ok(ReputationError::AppealWindowClosed)));
}

// ── Moderation-queue bound (#976) ───────────────────────────────────────────

#[test]
fn test_per_review_report_cap_is_enforced() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin) = setup(&env);

    let artist = Address::generate(&env);
    let review_id = make_bytes(&env, "cap");
    client.submit_review(&review_id, &artist, &Address::generate(&env), &30u32, &make_string(&env, "ok"));

    // Each report moves the review to UnderReview and moderation back to
    // Cleared makes it reportable again, so the per-review cap is reachable.
    let mut reports = 0u32;
    while reports < crate::types::MAX_REPORTS_PER_REVIEW {
        client.report_review(&review_id, &Address::generate(&env), &ReportReason::Spam, &make_string(&env, "s"));
        client.moderate_review(&review_id, &ReviewStatus::Cleared, &make_string(&env, "cleared"));
        reports += 1;
    }

    let result = client.try_report_review(
        &review_id,
        &Address::generate(&env),
        &ReportReason::Spam,
        &make_string(&env, "s"),
    );
    assert_eq!(result, Err(Ok(ReputationError::ReportLimitReached)));
}

#[test]
fn test_queue_cap_rejects_new_reports() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin) = setup(&env);

    let artist = Address::generate(&env);
    for i in 0..MAX_QUEUE_SIZE {
        let id = Bytes::from_slice(&env, std::format!("q{i}").as_bytes());
        client.submit_review(&id, &artist, &Address::generate(&env), &30u32, &make_string(&env, "ok"));
        client.report_review(&id, &Address::generate(&env), &ReportReason::Spam, &make_string(&env, "s"));
    }
    assert_eq!(client.get_moderation_queue_size(), MAX_QUEUE_SIZE);

    let extra = Bytes::from_slice(&env, b"q-extra");
    client.submit_review(&extra, &artist, &Address::generate(&env), &30u32, &make_string(&env, "ok"));
    let result = client.try_report_review(
        &extra,
        &Address::generate(&env),
        &ReportReason::Spam,
        &make_string(&env, "s"),
    );
    assert_eq!(result, Err(Ok(ReputationError::QueueFull)));
}

// ── Retention for active moderation (#054) ──────────────────────────────────

#[test]
fn test_report_and_moderation_refresh_review_retention() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin, contract_id) = setup_with_id(&env);

    let artist = Address::generate(&env);
    let reviewer = Address::generate(&env);
    let reporter = Address::generate(&env);
    let review_id = make_bytes(&env, "ttl-review");
    let review_key = DataKey::Review(review_id.clone());

    client.submit_review(&review_id, &artist, &reviewer, &40u32, &make_string(&env, "ok"));
    assert!(ttl_of(&env, &contract_id, &review_key) >= crate::REPUTATION_TTL_LEDGERS);

    // A report rewrites the review; its retention must not fall back to the
    // host default while moderation is still open.
    env.ledger().with_mut(|l| l.sequence_number += 100);
    client.report_review(&review_id, &reporter, &ReportReason::Spam, &make_string(&env, "s"));
    assert!(ttl_of(&env, &contract_id, &review_key) >= crate::REPUTATION_TTL_LEDGERS);
    assert!(
        ttl_of(&env, &contract_id, &DataKey::Report(review_id.clone(), 0))
            >= crate::REPUTATION_TTL_LEDGERS
    );

    // Moderation rewrites the review again and records the decision.
    env.ledger().with_mut(|l| l.sequence_number += 100);
    client.moderate_review(&review_id, &ReviewStatus::Removed, &make_string(&env, "removed"));
    assert!(ttl_of(&env, &contract_id, &review_key) >= crate::REPUTATION_TTL_LEDGERS);
    assert!(
        ttl_of(&env, &contract_id, &DataKey::ModerationEntry(review_id.clone(), 0))
            >= crate::REPUTATION_TTL_LEDGERS
    );
}

#[test]
fn test_filing_and_escalating_an_appeal_refreshes_retention() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin, contract_id) = setup_with_id(&env);

    let artist = Address::generate(&env);
    let review_id = make_bytes(&env, "ttl-appeal");
    let review_key = DataKey::Review(review_id.clone());
    let appeal_key = DataKey::Appeal(review_id.clone());

    client.submit_review(&review_id, &artist, &Address::generate(&env), &30u32, &make_string(&env, "ok"));
    client.report_review(&review_id, &Address::generate(&env), &ReportReason::Spam, &make_string(&env, "s"));
    client.moderate_review(&review_id, &ReviewStatus::Removed, &make_string(&env, "removed"));

    env.ledger().with_mut(|l| l.sequence_number += 100);
    client.file_appeal(&review_id, &artist, &make_string(&env, "unfair"));
    assert!(ttl_of(&env, &contract_id, &review_key) >= crate::REPUTATION_TTL_LEDGERS);
    assert!(ttl_of(&env, &contract_id, &appeal_key) >= crate::REPUTATION_TTL_LEDGERS);

    env.ledger().with_mut(|l| l.sequence_number += 100);
    client.escalate_appeal(&review_id, &artist);
    assert!(ttl_of(&env, &contract_id, &appeal_key) >= crate::REPUTATION_TTL_LEDGERS);
}

#[test]
fn test_resolving_an_appeal_refreshes_retention() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin, contract_id) = setup_with_id(&env);

    let artist = Address::generate(&env);
    let review_id = make_bytes(&env, "ttl-resolve");
    let review_key = DataKey::Review(review_id.clone());
    let appeal_key = DataKey::Appeal(review_id.clone());

    client.submit_review(&review_id, &artist, &Address::generate(&env), &30u32, &make_string(&env, "ok"));
    client.report_review(&review_id, &Address::generate(&env), &ReportReason::Spam, &make_string(&env, "s"));
    client.moderate_review(&review_id, &ReviewStatus::Removed, &make_string(&env, "removed"));
    client.file_appeal(&review_id, &artist, &make_string(&env, "unfair"));

    env.ledger().with_mut(|l| l.sequence_number += 100);
    client.resolve_appeal(&review_id, &AppealStatus::Upheld);
    assert!(ttl_of(&env, &contract_id, &appeal_key) >= crate::REPUTATION_TTL_LEDGERS);
    assert!(ttl_of(&env, &contract_id, &review_key) >= crate::REPUTATION_TTL_LEDGERS);
}

// ── Error-code stability ────────────────────────────────────────────────────

#[test]
fn test_error_codes_are_stable() {
    assert_eq!(ReputationError::AlreadyInitialized as u32, 1);
    assert_eq!(ReputationError::NotInitialized as u32, 2);
    assert_eq!(ReputationError::Unauthorized as u32, 3);
    assert_eq!(ReputationError::ReviewNotFound as u32, 4);
    assert_eq!(ReputationError::AlreadyReported as u32, 5);
    assert_eq!(ReputationError::InvalidRating as u32, 6);
    assert_eq!(ReputationError::InvalidStatus as u32, 7);
    assert_eq!(ReputationError::AppealAlreadyExists as u32, 8);
    assert_eq!(ReputationError::AppealNotFound as u32, 9);
    assert_eq!(ReputationError::ArithmeticOverflow as u32, 10);
    assert_eq!(ReputationError::DuplicateReview as u32, 11);
    assert_eq!(ReputationError::CommentTooLong as u32, 12);
    assert_eq!(ReputationError::AppealWindowClosed as u32, 13);
    assert_eq!(ReputationError::QueueFull as u32, 14);
    assert_eq!(ReputationError::ReportLimitReached as u32, 15);
    assert_eq!(ReputationError::InvalidAppealWindow as u32, 16);
}
