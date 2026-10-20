use serde_json::to_string;
use steerbox::adaptive_routing::{
    AdaptationAction, AdaptationContext, AdaptationDecision, EffectLevel, QualityAssessment,
    QualitySignal, QualityStatus,
};

fn context(
    current: EffectLevel,
    maximum: EffectLevel,
    assessment: QualityAssessment,
) -> AdaptationContext {
    AdaptationContext {
        current_effect_level: current,
        maximum_effect_level: maximum,
        assessment,
        remaining_attempts: 2,
        cooldown_active: false,
        budget_allows_upgrade: true,
        fallback_available: true,
    }
}

fn assessment(status: QualityStatus, signal: QualitySignal) -> QualityAssessment {
    QualityAssessment::new(status, signal)
}

#[test]
fn effect_levels_are_ordered_and_upgrade_one_step_at_a_time() {
    assert!(EffectLevel::Fast < EffectLevel::Balanced);
    assert!(EffectLevel::Balanced < EffectLevel::Strong);
    assert!(EffectLevel::Strong < EffectLevel::XHigh);
    assert_eq!(EffectLevel::Balanced.next(), Some(EffectLevel::Strong));
    assert_eq!(EffectLevel::XHigh.next(), None);
}

#[test]
fn needs_upgrade_moves_balanced_to_strong_without_changing_security_fields() {
    let decision = AdaptationDecision::from_context(context(
        EffectLevel::Balanced,
        EffectLevel::XHigh,
        assessment(
            QualityStatus::NeedsUpgrade,
            QualitySignal::VerificationFailed,
        ),
    ))
    .expect("bounded upgrade decision");

    assert_eq!(decision.action, AdaptationAction::Upgrade);
    assert_eq!(decision.from_effect_level, EffectLevel::Balanced);
    assert_eq!(decision.to_effect_level, Some(EffectLevel::Strong));
    assert!(!decision.creates_new_attempt);
    assert!(decision.reason_code.contains("verification"));

    let encoded = to_string(&decision).expect("decision serializes");
    for forbidden in ["endpoint", "auth_ref", "api_key", "token", "password"] {
        assert!(!encoded.contains(forbidden));
    }
}

#[test]
fn cooldown_or_budget_blocks_upgrade_without_retry_loop() {
    let mut blocked = context(
        EffectLevel::Balanced,
        EffectLevel::XHigh,
        assessment(QualityStatus::NeedsUpgrade, QualitySignal::LowQuality),
    );
    blocked.cooldown_active = true;
    let decision = AdaptationDecision::from_context(blocked).expect("bounded stop decision");
    assert_eq!(decision.action, AdaptationAction::Stop);
    assert_eq!(decision.to_effect_level, None);
    assert!(!decision.creates_new_attempt);

    let mut no_budget = context(
        EffectLevel::Balanced,
        EffectLevel::XHigh,
        assessment(QualityStatus::NeedsUpgrade, QualitySignal::LowQuality),
    );
    no_budget.budget_allows_upgrade = false;
    let decision = AdaptationDecision::from_context(no_budget).expect("bounded stop decision");
    assert_eq!(decision.action, AdaptationAction::Stop);
    assert_eq!(decision.to_effect_level, None);
}

#[test]
fn retryable_quality_uses_new_attempt_only_when_fallback_is_available() {
    let decision = AdaptationDecision::from_context(context(
        EffectLevel::Strong,
        EffectLevel::XHigh,
        assessment(QualityStatus::Retryable, QualitySignal::ProviderTimeout),
    ))
    .expect("fallback decision");
    assert_eq!(decision.action, AdaptationAction::Fallback);
    assert!(decision.creates_new_attempt);

    let mut no_fallback = context(
        EffectLevel::Strong,
        EffectLevel::XHigh,
        assessment(QualityStatus::Retryable, QualitySignal::ProviderTimeout),
    );
    no_fallback.fallback_available = false;
    let decision = AdaptationDecision::from_context(no_fallback).expect("bounded stop decision");
    assert_eq!(decision.action, AdaptationAction::Stop);
    assert!(!decision.creates_new_attempt);
}

#[test]
fn blocked_quality_never_creates_attempt_or_upgrade() {
    let decision = AdaptationDecision::from_context(context(
        EffectLevel::Fast,
        EffectLevel::XHigh,
        assessment(QualityStatus::Blocked, QualitySignal::SecurityBoundary),
    ))
    .expect("blocked decision");
    assert_eq!(decision.action, AdaptationAction::Stop);
    assert_eq!(decision.to_effect_level, None);
    assert!(!decision.creates_new_attempt);
}

#[test]
fn security_signal_is_blocked_even_if_status_is_misclassified() {
    let decision = AdaptationDecision::from_context(context(
        EffectLevel::Balanced,
        EffectLevel::XHigh,
        assessment(QualityStatus::NeedsUpgrade, QualitySignal::SecurityBoundary),
    ))
    .expect("security signal must produce a bounded decision");
    assert_eq!(decision.action, AdaptationAction::Stop);
    assert_eq!(decision.reason_code, "security_or_policy_blocked");
    assert!(!decision.creates_new_attempt);
}

#[test]
fn upgrade_requires_remaining_attempt_budget() {
    let mut exhausted = context(
        EffectLevel::Balanced,
        EffectLevel::XHigh,
        assessment(QualityStatus::NeedsUpgrade, QualitySignal::LowQuality),
    );
    exhausted.remaining_attempts = 0;
    let decision = AdaptationDecision::from_context(exhausted).expect("bounded stop decision");
    assert_eq!(decision.action, AdaptationAction::Stop);
    assert_eq!(decision.reason_code, "retry_budget_exhausted");
}
