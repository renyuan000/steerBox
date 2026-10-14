use steerbox::memory::{
    MemoryCandidate, MemoryCandidateState, MemoryCatalog, MemoryConfidence, MemoryError,
    MemoryProvenance, MemoryQuery, MemoryScope,
};
use steerbox::redaction::DEFAULT_MAX_INPUT_BYTES;

fn provenance() -> MemoryProvenance {
    MemoryProvenance {
        producer: "test-fixture".into(),
        source_kind: "verification".into(),
    }
}

fn candidate(
    candidate_id: &str,
    scope: MemoryScope,
    project_id: &str,
    goal_id: Option<&str>,
    task_id: Option<&str>,
    state: MemoryCandidateState,
) -> MemoryCandidate {
    MemoryCandidate::new(
        candidate_id,
        scope,
        project_id,
        goal_id,
        task_id,
        "verified local test command",
        provenance(),
        vec![format!("event:{candidate_id}")],
        MemoryConfidence::Medium,
        state,
    )
    .expect("candidate is valid")
}

#[test]
fn memory_candidate_validates_provenance_and_scope_metadata() {
    let candidate = candidate(
        "candidate-project",
        MemoryScope::Project,
        "project-a",
        None,
        None,
        MemoryCandidateState::New,
    );

    assert_eq!(candidate.schema_version, 1);
    assert_eq!(candidate.project_id, "project-a");
    assert_eq!(candidate.source_event_refs, vec!["event:candidate-project"]);
    assert_eq!(candidate.lifecycle_state, MemoryCandidateState::New);
}

#[test]
fn memory_candidate_rejects_incomplete_provenance_and_scope() {
    let missing_event = MemoryCandidate::new(
        "candidate-missing-event",
        MemoryScope::Project,
        "project-a",
        None,
        None,
        "claim",
        provenance(),
        Vec::new(),
        MemoryConfidence::Low,
        MemoryCandidateState::New,
    )
    .expect_err("source event refs are required");
    assert_eq!(
        missing_event,
        MemoryError::InvalidField {
            field: "source_event_refs".into(),
            reason: "must contain at least one non-empty reference".into(),
        }
    );

    let goal_without_id = MemoryCandidate::new(
        "candidate-goal-without-id",
        MemoryScope::Goal,
        "project-a",
        None,
        None,
        "claim",
        provenance(),
        vec!["event:1".into()],
        MemoryConfidence::Low,
        MemoryCandidateState::New,
    )
    .expect_err("goal scope requires a goal id");
    assert_eq!(
        goal_without_id,
        MemoryError::InvalidField {
            field: "goal_id".into(),
            reason: "required for goal scope".into(),
        }
    );

    let task_without_id = MemoryCandidate::new(
        "candidate-task-without-id",
        MemoryScope::Task,
        "project-a",
        Some("goal-a"),
        None,
        "claim",
        provenance(),
        vec!["event:1".into()],
        MemoryConfidence::Low,
        MemoryCandidateState::New,
    )
    .expect_err("task scope requires a task id");
    assert_eq!(
        task_without_id,
        MemoryError::InvalidField {
            field: "task_id".into(),
            reason: "required for task scope".into(),
        }
    );
}

#[test]
fn memory_catalog_retrieves_deterministically_with_project_isolation() {
    let mut catalog = MemoryCatalog::default();
    catalog
        .insert(candidate(
            "z-goal",
            MemoryScope::Goal,
            "project-a",
            Some("goal-a"),
            None,
            MemoryCandidateState::Reviewing,
        ))
        .expect("goal candidate inserts");
    catalog
        .insert(candidate(
            "a-project",
            MemoryScope::Project,
            "project-a",
            None,
            None,
            MemoryCandidateState::Accepted,
        ))
        .expect("project candidate inserts");
    catalog
        .insert(candidate(
            "m-task",
            MemoryScope::Task,
            "project-a",
            Some("goal-a"),
            Some("task-a"),
            MemoryCandidateState::New,
        ))
        .expect("task candidate inserts");
    catalog
        .insert(candidate(
            "other-project",
            MemoryScope::Project,
            "project-b",
            None,
            None,
            MemoryCandidateState::Accepted,
        ))
        .expect("other project candidate inserts");
    catalog
        .insert(candidate(
            "rejected",
            MemoryScope::Project,
            "project-a",
            None,
            None,
            MemoryCandidateState::Rejected,
        ))
        .expect("rejected candidate inserts");

    let query =
        MemoryQuery::new("project-a", Some("goal-a"), Some("task-a")).expect("query is valid");
    let result = catalog.retrieve(&query);
    assert_eq!(
        result
            .iter()
            .map(|candidate| candidate.candidate_id.as_str())
            .collect::<Vec<_>>(),
        vec!["a-project", "m-task", "z-goal"]
    );

    let other_project =
        MemoryQuery::new("project-b", Some("goal-a"), Some("task-a")).expect("query is valid");
    let result = catalog.retrieve(&other_project);
    assert_eq!(
        result
            .iter()
            .map(|candidate| candidate.candidate_id.as_str())
            .collect::<Vec<_>>(),
        vec!["other-project"]
    );
}

#[test]
fn memory_catalog_rejects_duplicate_ids_and_invalid_queries() {
    let mut catalog = MemoryCatalog::default();
    catalog
        .insert(candidate(
            "duplicate",
            MemoryScope::Project,
            "project-a",
            None,
            None,
            MemoryCandidateState::New,
        ))
        .expect("first candidate inserts");
    let duplicate = catalog
        .insert(candidate(
            "duplicate",
            MemoryScope::Project,
            "project-a",
            None,
            None,
            MemoryCandidateState::New,
        ))
        .expect_err("duplicate candidate ids are rejected");
    assert_eq!(
        duplicate,
        MemoryError::DuplicateCandidate("duplicate".into())
    );

    let invalid_query = MemoryQuery::new(" ", None, None).expect_err("project is required");
    assert_eq!(
        invalid_query,
        MemoryError::InvalidField {
            field: "project_id".into(),
            reason: "must not be blank".into(),
        }
    );
}

#[test]
fn memory_candidate_round_trips_without_changing_scope_metadata() {
    let candidate = candidate(
        "candidate-round-trip",
        MemoryScope::Task,
        "project-a",
        Some("goal-a"),
        Some("task-a"),
        MemoryCandidateState::Accepted,
    );
    let encoded = serde_json::to_value(&candidate).expect("candidate serializes");
    let decoded: MemoryCandidate = serde_json::from_value(encoded).expect("candidate decodes");
    assert_eq!(decoded, candidate);
}

#[test]
fn memory_candidate_rejects_unsupported_schema_after_deserialization() {
    for version in [0, 2] {
        let mut encoded = serde_json::to_value(candidate(
            "unsupported-schema",
            MemoryScope::Project,
            "project-a",
            None,
            None,
            MemoryCandidateState::New,
        ))
        .expect("candidate serializes");
        encoded["schema_version"] = serde_json::json!(version);
        let decoded: MemoryCandidate = serde_json::from_value(encoded).expect("schema decodes");
        assert!(matches!(
            decoded.validate(),
            Err(MemoryError::InvalidField { field, .. }) if field == "schema_version"
        ));
        let mut catalog = MemoryCatalog::default();
        assert!(matches!(
            catalog.insert(decoded),
            Err(MemoryError::InvalidField { field, .. }) if field == "schema_version"
        ));
        assert!(catalog
            .retrieve(&MemoryQuery::new("project-a", None, None).expect("valid query"))
            .is_empty());
    }
}

#[test]
fn memory_candidate_constructor_redacts_sensitive_claim_summaries() {
    let candidate = MemoryCandidate::new(
        "redacted-summary",
        MemoryScope::Project,
        "project-a",
        None,
        None,
        "fixture password=synthetic-password token=synthetic-token",
        provenance(),
        vec!["event:summary".into()],
        MemoryConfidence::Low,
        MemoryCandidateState::New,
    )
    .expect("candidate constructor redacts locally");
    assert!(!candidate.claim_summary.contains("synthetic-password"));
    assert!(!candidate.claim_summary.contains("synthetic-token"));
    assert!(candidate.claim_summary.contains("[REDACTED:PASSWORD]"));
    assert!(candidate.claim_summary.contains("[REDACTED:TOKEN]"));
    candidate.validate().expect("redacted candidate validates");
    let mut catalog = MemoryCatalog::default();
    catalog
        .insert(candidate)
        .expect("redacted candidate inserts");
    let encoded = serde_json::to_string(
        &catalog.retrieve(&MemoryQuery::new("project-a", None, None).expect("valid query")),
    )
    .expect("retrieved metadata serializes");
    assert!(!encoded.contains("synthetic-password"));
    assert!(!encoded.contains("synthetic-token"));
}

#[test]
fn memory_catalog_rejects_unredacted_deserialized_summary_without_echoing_it() {
    let mut encoded = serde_json::to_value(candidate(
        "unsafe-deserialized-summary",
        MemoryScope::Project,
        "project-a",
        None,
        None,
        MemoryCandidateState::New,
    ))
    .expect("candidate serializes");
    encoded["claim_summary"] = serde_json::json!("password=synthetic-deserialized-password");
    let decoded: MemoryCandidate = serde_json::from_value(encoded).expect("candidate decodes");
    let error = decoded.validate().expect_err("raw summary is not valid");
    assert!(matches!(
        &error,
        MemoryError::InvalidField { field, .. } if field == "claim_summary"
    ));
    assert!(!error
        .to_string()
        .contains("synthetic-deserialized-password"));
    let mut catalog = MemoryCatalog::default();
    assert_eq!(catalog.insert(decoded), Err(error));
    assert!(catalog
        .retrieve(&MemoryQuery::new("project-a", None, None).expect("valid query"))
        .is_empty());
}

#[test]
fn memory_candidate_summary_redaction_limit_fails_closed() {
    let oversized_summary = "x".repeat(DEFAULT_MAX_INPUT_BYTES + 1);
    let mut oversized = candidate(
        "oversized-summary",
        MemoryScope::Project,
        "project-a",
        None,
        None,
        MemoryCandidateState::New,
    );
    oversized.claim_summary = oversized_summary.clone();
    assert!(matches!(
        oversized.validate(),
        Err(MemoryError::InvalidField { field, .. }) if field == "claim_summary"
    ));
    let mut catalog = MemoryCatalog::default();
    assert!(catalog.insert(oversized).is_err());
    assert!(MemoryCandidate::new(
        "oversized-constructor-summary",
        MemoryScope::Project,
        "project-a",
        None,
        None,
        oversized_summary,
        provenance(),
        vec!["event:oversized".into()],
        MemoryConfidence::Low,
        MemoryCandidateState::New,
    )
    .is_err());
}

#[test]
fn memory_candidate_rejects_duplicate_source_event_references() {
    let error = MemoryCandidate::new(
        "duplicate-source-refs",
        MemoryScope::Project,
        "project-a",
        None,
        None,
        "verified local test command",
        provenance(),
        vec!["event:one".into(), "event:one".into()],
        MemoryConfidence::Medium,
        MemoryCandidateState::New,
    )
    .expect_err("duplicate source references are rejected");
    assert!(matches!(
        error,
        MemoryError::InvalidField { field, .. } if field == "source_event_refs"
    ));
}

#[test]
fn memory_catalog_rejects_illegal_deserialized_scope_combinations() {
    for (scope, goal_id, task_id) in [
        ("project", Some("goal-a"), None),
        ("project", None, Some("task-a")),
        ("goal", None, None),
        ("goal", Some(" "), None),
        ("goal", Some("goal-a"), Some("task-a")),
        ("task", Some("goal-a"), None),
        ("task", Some("goal-a"), Some(" ")),
        ("task", Some(" "), Some("task-a")),
    ] {
        let mut encoded = serde_json::to_value(candidate(
            "illegal-scope",
            MemoryScope::Project,
            "project-a",
            None,
            None,
            MemoryCandidateState::New,
        ))
        .expect("candidate serializes");
        encoded["scope"] = serde_json::json!(scope);
        encoded["goal_id"] = serde_json::json!(goal_id);
        encoded["task_id"] = serde_json::json!(task_id);
        let decoded: MemoryCandidate = serde_json::from_value(encoded).expect("candidate decodes");
        assert!(decoded.validate().is_err());
        let mut catalog = MemoryCatalog::default();
        assert!(catalog.insert(decoded).is_err());
        assert!(catalog
            .retrieve(&MemoryQuery::new("project-a", None, None).expect("valid query"))
            .is_empty());
    }
    assert!(serde_json::from_str::<MemoryScope>("\"global\"").is_err());
    assert!(serde_json::from_str::<MemoryScope>("\"cross_project\"").is_err());
}

#[test]
fn memory_catalog_filters_goal_and_task_before_returning_candidate_metadata() {
    let mut catalog = MemoryCatalog::default();
    for (candidate_id, scope, goal_id, task_id) in [
        ("project", MemoryScope::Project, None, None),
        ("goal-a", MemoryScope::Goal, Some("goal-a"), None),
        ("goal-b", MemoryScope::Goal, Some("goal-b"), None),
        ("task-aa", MemoryScope::Task, Some("goal-a"), Some("task-a")),
        ("task-ab", MemoryScope::Task, Some("goal-a"), Some("task-b")),
        ("task-ba", MemoryScope::Task, Some("goal-b"), Some("task-a")),
        ("task-only", MemoryScope::Task, None, Some("task-a")),
    ] {
        catalog
            .insert(candidate(
                candidate_id,
                scope,
                "project-a",
                goal_id,
                task_id,
                MemoryCandidateState::New,
            ))
            .expect("scoped candidate inserts");
    }
    for (goal_id, task_id, expected) in [
        (None, None, vec!["project"]),
        (Some("goal-a"), None, vec!["goal-a", "project"]),
        (
            Some("goal-a"),
            Some("task-a"),
            vec!["goal-a", "project", "task-aa"],
        ),
        (
            Some("goal-a"),
            Some("task-b"),
            vec!["goal-a", "project", "task-ab"],
        ),
        (
            Some("goal-b"),
            Some("task-a"),
            vec!["goal-b", "project", "task-ba"],
        ),
        (None, Some("task-a"), vec!["project", "task-only"]),
        (Some("unknown-goal"), Some("task-a"), vec!["project"]),
    ] {
        let query = MemoryQuery::new("project-a", goal_id, task_id).expect("valid query");
        let result = catalog.retrieve(&query);
        assert_eq!(
            result
                .iter()
                .map(|candidate| candidate.candidate_id.as_str())
                .collect::<Vec<_>>(),
            expected
        );
        assert!(result
            .iter()
            .all(|candidate| candidate.lifecycle_state == MemoryCandidateState::New));
    }
}

#[test]
fn memory_candidate_retrieval_preserves_states_without_accepting_candidates() {
    let mut catalog = MemoryCatalog::default();
    for (candidate_id, state) in [
        ("a-new", MemoryCandidateState::New),
        ("b-reviewing", MemoryCandidateState::Reviewing),
        ("c-accepted", MemoryCandidateState::Accepted),
        ("d-rejected", MemoryCandidateState::Rejected),
        ("e-superseded", MemoryCandidateState::Superseded),
    ] {
        catalog
            .insert(candidate(
                candidate_id,
                MemoryScope::Project,
                "project-a",
                None,
                None,
                state,
            ))
            .expect("candidate metadata inserts");
    }
    let query = MemoryQuery::new("project-a", None, None).expect("valid query");
    let result = catalog.retrieve(&query);
    assert_eq!(
        result
            .iter()
            .map(|candidate| (candidate.candidate_id.as_str(), candidate.lifecycle_state))
            .collect::<Vec<_>>(),
        vec![
            ("a-new", MemoryCandidateState::New),
            ("b-reviewing", MemoryCandidateState::Reviewing),
            ("c-accepted", MemoryCandidateState::Accepted),
        ]
    );
    assert_eq!(catalog.retrieve(&query), result);
}
