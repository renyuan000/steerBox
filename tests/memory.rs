use steerbox::domain::EventEnvelope;
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

fn source_event(
    event_id: &str,
    project_id: Option<&str>,
    goal_id: Option<&str>,
    task_id: Option<&str>,
) -> EventEnvelope {
    let mut event = EventEnvelope::new(
        "memory-source",
        event_id,
        "verification_completed",
        "source-fixture",
        "source-fixture",
        serde_json::json!({}),
    )
    .expect("source envelope constructs");
    event.project_id = project_id.map(str::to_owned);
    event.goal_id = goal_id.map(str::to_owned);
    event.task_id = task_id.map(str::to_owned);
    event
}

#[test]
fn memory_source_validation_accepts_project_goal_and_task_scopes() {
    for (scope, goal_id, task_id, event_goal, event_task) in [
        (MemoryScope::Project, None, None, None, None),
        (
            MemoryScope::Project,
            None,
            None,
            Some("any-goal"),
            Some("any-task"),
        ),
        (
            MemoryScope::Goal,
            Some("goal-a"),
            None,
            Some("goal-a"),
            Some("any-task"),
        ),
        (
            MemoryScope::Task,
            Some("goal-a"),
            Some("task-a"),
            Some("goal-a"),
            Some("task-a"),
        ),
        (
            MemoryScope::Task,
            None,
            Some("task-a"),
            Some("any-goal"),
            Some("task-a"),
        ),
        (
            MemoryScope::Task,
            None,
            Some("task-a"),
            None,
            Some("task-a"),
        ),
    ] {
        let mut candidate = candidate(
            "scoped-source",
            scope,
            "project-a",
            goal_id,
            task_id,
            MemoryCandidateState::New,
        );
        candidate.source_event_refs = vec!["event:first".into(), "event:second".into()];
        let events = [
            source_event("event:second", Some("project-a"), event_goal, event_task),
            source_event("unrelated", Some("project-b"), None, None),
            source_event("event:first", Some("project-a"), event_goal, event_task),
        ];
        candidate
            .validate_source_events(&events)
            .expect("matching source identities validate");
    }
}

#[test]
fn memory_source_validation_requires_every_reference_to_exist_exactly_once() {
    let mut candidate = candidate(
        "missing-source",
        MemoryScope::Project,
        "project-a",
        None,
        None,
        MemoryCandidateState::New,
    );
    candidate.source_event_refs = vec!["event:first".into(), "event:second".into()];
    let first = source_event("event:first", Some("project-a"), None, None);
    for events in [
        vec![],
        vec![first.clone()],
        vec![
            first.clone(),
            source_event("EVENT:SECOND", Some("project-a"), None, None),
        ],
    ] {
        assert!(candidate.validate_source_events(&events).is_err());
    }
    let second = source_event("event:second", Some("project-a"), None, None);
    for duplicate in [
        second.clone(),
        source_event("event:second", Some("project-b"), None, None),
    ] {
        assert!(candidate
            .validate_source_events(&[first.clone(), second.clone(), duplicate])
            .is_err());
    }
    let unrelated = source_event("unreferenced", Some("project-b"), None, None);
    candidate
        .validate_source_events(&[first, second, unrelated.clone(), unrelated])
        .expect("unreferenced duplicate ids do not affect requested sources");
}

#[test]
fn memory_source_validation_rejects_missing_or_different_project_identity() {
    for scope in [MemoryScope::Project, MemoryScope::Goal, MemoryScope::Task] {
        let goal_id = (scope != MemoryScope::Project).then_some("goal-a");
        let task_id = (scope == MemoryScope::Task).then_some("task-a");
        let candidate = candidate(
            "project-source",
            scope,
            "project-a",
            goal_id,
            task_id,
            MemoryCandidateState::New,
        );
        for project in [
            None,
            Some("project-b"),
            Some("project-a "),
            Some("PROJECT-A"),
        ] {
            let event = source_event("event:project-source", project, goal_id, task_id);
            assert!(candidate.validate_source_events(&[event]).is_err());
        }
    }
}

#[test]
fn memory_source_validation_rejects_goal_and_task_scope_mismatches() {
    for (scope, candidate_goal, candidate_task, event_goal, event_task) in [
        (MemoryScope::Goal, Some("goal-a"), None, None, None),
        (
            MemoryScope::Goal,
            Some("goal-a"),
            None,
            Some("goal-b"),
            None,
        ),
        (
            MemoryScope::Task,
            Some("goal-a"),
            Some("task-a"),
            None,
            Some("task-a"),
        ),
        (
            MemoryScope::Task,
            Some("goal-a"),
            Some("task-a"),
            Some("goal-b"),
            Some("task-a"),
        ),
        (
            MemoryScope::Task,
            Some("goal-a"),
            Some("task-a"),
            Some("goal-a"),
            None,
        ),
        (
            MemoryScope::Task,
            Some("goal-a"),
            Some("task-a"),
            Some("goal-a"),
            Some("task-b"),
        ),
        (
            MemoryScope::Task,
            None,
            Some("task-a"),
            Some("any-goal"),
            Some("task-b"),
        ),
        (MemoryScope::Task, None, Some("task-a"), None, None),
    ] {
        let candidate = candidate(
            "scoped-source",
            scope,
            "project-a",
            candidate_goal,
            candidate_task,
            MemoryCandidateState::New,
        );
        let event = source_event(
            "event:scoped-source",
            Some("project-a"),
            event_goal,
            event_task,
        );
        assert!(candidate.validate_source_events(&[event]).is_err());
    }
}

#[test]
fn memory_source_validation_checks_candidate_before_looking_up_events() {
    let valid = candidate(
        "invalid-source",
        MemoryScope::Project,
        "project-a",
        None,
        None,
        MemoryCandidateState::New,
    );
    let mut unsupported = valid.clone();
    unsupported.schema_version = 2;
    let mut raw_summary = valid.clone();
    raw_summary.claim_summary = "password=synthetic-invalid-candidate".into();
    let mut no_refs = valid.clone();
    no_refs.source_event_refs.clear();
    let mut illegal_scope = valid;
    illegal_scope.scope = MemoryScope::Goal;
    for candidate in [unsupported, raw_summary, no_refs, illegal_scope] {
        let expected = candidate.validate().expect_err("candidate is invalid");
        assert_eq!(candidate.validate_source_events(&[]), Err(expected));
    }
}

#[test]
fn memory_source_validation_only_reads_identity_fields_not_payload_or_schema() {
    let candidate = candidate(
        "payload-source",
        MemoryScope::Project,
        "project-a",
        None,
        None,
        MemoryCandidateState::New,
    );
    let mut event = source_event("event:payload-source", Some("project-a"), None, None);
    event.schema_version = 0;
    event.event_type.clear();
    event.payload = serde_json::json!({"password": "synthetic-source-payload", "token": "synthetic-source-token"});
    let before = event.clone();
    candidate
        .validate_source_events(std::slice::from_ref(&event))
        .expect("only source identity metadata is checked");
    assert!(event == before);
}

#[test]
fn memory_source_validation_errors_do_not_echo_sensitive_identity_or_payload() {
    let mut candidate = candidate(
        "synthetic-candidate-id",
        MemoryScope::Task,
        "project-a",
        Some("goal-a"),
        Some("task-a"),
        MemoryCandidateState::New,
    );
    candidate.source_event_refs = vec!["event:synthetic-sensitive-reference".into()];
    let mut matching = source_event(
        "event:synthetic-sensitive-reference",
        Some("project-a"),
        Some("goal-a"),
        Some("task-a"),
    );
    matching.payload = serde_json::json!({"password": "synthetic-payload-secret"});
    let mut wrong_project = matching.clone();
    wrong_project.project_id = Some("synthetic-private-project".into());
    let mut wrong_goal = matching.clone();
    wrong_goal.goal_id = Some("synthetic-private-goal".into());
    let mut wrong_task = matching.clone();
    wrong_task.task_id = Some("synthetic-private-task".into());
    for events in [
        vec![],
        vec![matching.clone(), matching],
        vec![wrong_project],
        vec![wrong_goal],
        vec![wrong_task],
    ] {
        let error = candidate
            .validate_source_events(&events)
            .expect_err("source identity is rejected");
        assert!(!error.to_string().contains("synthetic"));
        assert!(!format!("{error:?}").contains("synthetic"));
    }
}

#[test]
fn memory_source_validation_does_not_change_catalog_or_promote_candidates() {
    let candidate = candidate(
        "catalog-source",
        MemoryScope::Project,
        "project-a",
        None,
        None,
        MemoryCandidateState::New,
    );
    let mut catalog = MemoryCatalog::default();
    catalog
        .insert(candidate.clone())
        .expect("existing insert remains independent of source lookup");
    let before = catalog.clone();
    assert!(candidate.validate_source_events(&[]).is_err());
    assert_eq!(catalog, before);
    candidate
        .validate_source_events(&[source_event(
            "event:catalog-source",
            Some("project-a"),
            None,
            None,
        )])
        .expect("source identity matches");
    assert_eq!(catalog, before);
    assert_eq!(candidate.lifecycle_state, MemoryCandidateState::New);
    let query = MemoryQuery::new("project-a", None, None).unwrap();
    assert_eq!(catalog.retrieve(&query), vec![candidate]);
}
