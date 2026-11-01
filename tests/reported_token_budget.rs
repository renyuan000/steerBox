use steerbox::reported_token_budget::{ReportedTokenBudget, ReportedTokenBudgetError};

#[test]
fn reported_usage_is_checked_idempotent_and_not_request_replay() {
    let mut budget = ReportedTokenBudget::new(10);
    assert!(budget.check_send("first").is_ok());
    budget.settle("first", Some(2), Some(3)).unwrap();
    budget.settle("first", Some(2), Some(3)).unwrap();
    assert_eq!(budget.known_input_tokens(), 2);
    assert_eq!(budget.known_output_tokens(), 3);
    assert_eq!(budget.known_total_tokens(), 5);
    assert_eq!(
        budget.check_send("first"),
        Err(ReportedTokenBudgetError::CallAlreadySettled)
    );
    assert!(budget.check_send("second").is_ok());
    assert!(budget.settle("first", Some(3), Some(2)).is_err());
    assert!(budget.check_send("second").is_err());
}

#[test]
fn zero_limit_and_exact_or_exceeded_limits_prevent_next_send() {
    assert!(ReportedTokenBudget::new(0).check_send("first").is_err());
    for reported in [10, 11] {
        let mut budget = ReportedTokenBudget::new(10);
        budget.settle("first", Some(reported), Some(0)).unwrap();
        assert_eq!(budget.known_total_tokens(), reported);
        assert!(budget.check_send("next").is_err());
    }
    let mut budget = ReportedTokenBudget::new(1);
    budget.settle("zero", Some(0), Some(0)).unwrap();
    assert!(budget.check_send("next").is_ok());
}

#[test]
fn partial_unknown_preserves_known_facts_and_permanently_blocks() {
    for usage in [(None, Some(3)), (Some(3), None), (None, None)] {
        let mut budget = ReportedTokenBudget::new(10);
        assert!(budget.settle("first", usage.0, usage.1).is_err());
        assert_eq!(budget.records()[0].input_tokens(), usage.0);
        assert_eq!(budget.records()[0].output_tokens(), usage.1);
        assert!(budget.check_send("next").is_err());
        budget.settle("first", usage.0, usage.1).unwrap();
        assert!(budget.check_send("next").is_err());
    }
}

#[test]
fn per_call_and_cumulative_overflow_never_wrap() {
    let mut single = ReportedTokenBudget::new(u64::MAX);
    assert!(single.settle("first", Some(u64::MAX), Some(1)).is_err());
    assert!(single.check_send("next").is_err());
    let mut cumulative = ReportedTokenBudget::new(u64::MAX);
    cumulative
        .settle("first", Some(u64::MAX - 1), Some(0))
        .unwrap();
    assert!(cumulative.settle("next", Some(2), Some(0)).is_err());
    assert!(cumulative.check_send("third").is_err());
    let mut output = ReportedTokenBudget::new(u64::MAX);
    output.settle("first", Some(0), Some(u64::MAX - 1)).unwrap();
    assert!(output.settle("next", Some(0), Some(2)).is_err());
    assert!(output.check_send("third").is_err());
    for budget in [single, cumulative, output] {
        let restored: ReportedTokenBudget =
            serde_json::from_value(serde_json::to_value(&budget).unwrap()).unwrap();
        assert_eq!(restored, budget);
        assert!(restored.check_send("after-roundtrip").is_err());
    }
}

#[test]
fn serialized_state_round_trips_and_rejects_forged_subtotals() {
    let mut budget = ReportedTokenBudget::new(20);
    budget.settle("first", Some(3), Some(4)).unwrap();
    budget.block_audit_failure();
    let mut serialized = serde_json::to_value(&budget).unwrap();
    let restored: ReportedTokenBudget = serde_json::from_value(serialized.clone()).unwrap();
    assert_eq!(restored, budget);
    assert!(restored.check_send("next").is_err());
    serialized["known_input_tokens"] = serde_json::json!(0);
    assert!(serde_json::from_value::<ReportedTokenBudget>(serialized).is_err());
    let mut unknown = ReportedTokenBudget::new(20);
    assert!(unknown.settle("unknown", None, Some(4)).is_err());
    let mut serialized = serde_json::to_value(&unknown).unwrap();
    serialized["blocked"] = serde_json::Value::Null;
    assert!(serde_json::from_value::<ReportedTokenBudget>(serialized).is_err());
    let mut duplicated = serde_json::to_value(ReportedTokenBudget::new(20)).unwrap();
    duplicated["records"] = serde_json::json!([
        {"model_call_id":"same", "input_tokens":0, "output_tokens":0},
        {"model_call_id":"same", "input_tokens":0, "output_tokens":0}
    ]);
    assert!(serde_json::from_value::<ReportedTokenBudget>(duplicated).is_err());
}

#[test]
fn invalid_call_id_does_not_create_an_unserializable_state() {
    let mut budget = ReportedTokenBudget::new(20);
    assert!(budget.settle("", Some(0), Some(0)).is_err());
    let serialized = serde_json::to_value(&budget).unwrap();
    let restored: ReportedTokenBudget = serde_json::from_value(serialized).unwrap();
    assert_eq!(restored, budget);
    assert!(restored.records().is_empty());
}
