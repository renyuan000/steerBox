use std::collections::VecDeque;
use std::fs;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};
use steerbox::audit_store::{CheckpointStore, SideEffectLedger, TraceStore};
use steerbox::domain::{
    AttemptStatus, BudgetKind, GoalContract, HumanSteeringMode, LoopBudget, LoopStatus,
    TaskRunStatus,
};
use steerbox::event_log::EventLog;
use steerbox::loop_controller::{LoopControllerError, ModelActionRuntime, ReadOnlyLoopController};
use steerbox::memory::{
    MemoryCandidate, MemoryCandidateState, MemoryConfidence, MemoryPromotionDecision,
    MemoryProvenance, MemoryQuery, MemoryScope,
};
use steerbox::memory_store::MemoryStore;
use steerbox::model_adapter::{
    LocalFixtureModelAdapter, ModelAdapter, ModelAdapterError, ModelDecision, ModelRequest,
    ModelResponse, ModelResponseWithAction,
};
use steerbox::model_contracts::{ActionPathScope, ModelActionEnvelope, VerificationContract};
use steerbox::policy_gate::{PolicyDecision, PolicyGate};
use steerbox::stream_reporter::{ProgressEvent, ProgressSink, ProgressStatus, ProgressSubject};
use steerbox::tool_executor::{ReadOnlyToolExecutor, ToolCallInput, WorkspaceToolExecutor};
use steerbox::tool_registry::ToolRegistry;
use steerbox::user_command_inbox::{
    CommandDrainStopReason, CommandKind, CommandPollResult, CommandPoller, CommandRead,
    CommandStatus, NonBlockingCommandReader, UserCommand, UserCommandInbox, UserCommandInboxError,
};

fn test_data_dir() -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock is after epoch")
        .as_nanos();
    PathBuf::from(format!("/temp/steerbox/test-loop-{nonce}"))
}

fn runtime_memory_candidate(
    candidate_id: &str,
    project_id: &str,
    scope: MemoryScope,
    goal_id: Option<&str>,
    task_id: Option<&str>,
    lifecycle_state: MemoryCandidateState,
) -> MemoryCandidate {
    MemoryCandidate::new(
        candidate_id,
        scope,
        project_id,
        goal_id,
        task_id,
        format!("claim for {candidate_id} password=synthetic-memory-secret"),
        MemoryProvenance {
            producer: "loop-controller-test".into(),
            source_kind: "verification".into(),
        },
        vec![format!("event:{candidate_id}")],
        MemoryConfidence::Medium,
        lifecycle_state,
    )
    .expect("runtime memory candidate is valid")
}

struct RecordingSink(Arc<Mutex<Vec<ProgressEvent>>>);

impl ProgressSink for RecordingSink {
    fn report(&mut self, event: ProgressEvent) -> std::io::Result<()> {
        self.0.lock().expect("recording sink lock").push(event);
        Ok(())
    }
}

struct ActionFixtureAdapter {
    action: ModelActionEnvelope,
}

struct RecordingObservationAdapter {
    observations: Arc<Mutex<Vec<String>>>,
}

impl ModelAdapter for RecordingObservationAdapter {
    fn complete(&self, request: ModelRequest) -> Result<ModelResponse, ModelAdapterError> {
        self.observations
            .lock()
            .expect("observation lock")
            .push(request.observation);
        Ok(ModelResponse {
            decision: ModelDecision::ContinueObservation,
            summary: "recorded tool result".into(),
        })
    }
}

impl ModelAdapter for ActionFixtureAdapter {
    fn complete(&self, _request: ModelRequest) -> Result<ModelResponse, ModelAdapterError> {
        Ok(ModelResponse {
            decision: ModelDecision::ContinueObservation,
            summary: "fixture action proposed".into(),
        })
    }

    fn complete_with_action(
        &self,
        _request: ModelRequest,
    ) -> Result<ModelResponseWithAction, ModelAdapterError> {
        Ok(ModelResponseWithAction {
            decision: ModelDecision::ContinueObservation,
            summary: "fixture action proposed".into(),
            action: Some(self.action.clone()),
        })
    }
}

#[test]
fn controller_reports_safe_lifecycle_metadata_without_payloads() {
    let directory = test_data_dir();
    fs::create_dir_all(&directory).expect("test directory creates");
    let recorded = Arc::new(Mutex::new(Vec::new()));
    let mut controller = ReadOnlyLoopController::start_with_progress_sink(
        EventLog::open(directory.join("events.jsonl")).expect("event log opens"),
        GoalContract::new(
            "goal-progress",
            "project-progress",
            "inspect repository",
            vec![".".into()],
            Vec::new(),
            vec!["completed".into()],
        ),
        "loop-progress",
        "task-run-progress",
        LoopBudget::new(2, 0, 0, 1, 0, 1, 1, 1),
        RecordingSink(recorded.clone()),
    )
    .expect("loop starts with progress sink");

    controller
        .run_observation("step-progress", "secret observation must not stream")
        .expect("observation succeeds");
    controller.complete().expect("loop completes");

    let events = recorded.lock().expect("recorded events lock");
    assert_eq!(events[0].subject, ProgressSubject::Loop);
    assert_eq!(events[0].status, ProgressStatus::Started);
    assert!(events.iter().any(|event| {
        event.subject == ProgressSubject::Step && event.status == ProgressStatus::Started
    }));
    assert!(events.iter().any(|event| {
        event.subject == ProgressSubject::Step && event.status == ProgressStatus::Completed
    }));
    assert!(events.iter().any(|event| {
        event.subject == ProgressSubject::Loop && event.status == ProgressStatus::Completed
    }));
    assert!(events
        .iter()
        .all(|event| { event.summary.as_deref() != Some("secret observation must not stream") }));
}

#[test]
fn controller_retrieves_memory_candidates_only_when_explicitly_requested() {
    let directory = test_data_dir();
    fs::create_dir_all(&directory).expect("test directory creates");
    let event_path = directory.join("events.jsonl");
    let memory_store = MemoryStore::open(directory.join("memory.jsonl"));
    for candidate in [
        runtime_memory_candidate(
            "project-visible",
            "project-memory-runtime",
            MemoryScope::Project,
            None,
            None,
            MemoryCandidateState::New,
        ),
        runtime_memory_candidate(
            "goal-visible",
            "project-memory-runtime",
            MemoryScope::Goal,
            Some("goal-memory-runtime"),
            None,
            MemoryCandidateState::New,
        ),
        runtime_memory_candidate(
            "task-visible",
            "project-memory-runtime",
            MemoryScope::Task,
            Some("goal-memory-runtime"),
            Some("task-run-memory-runtime"),
            MemoryCandidateState::New,
        ),
        runtime_memory_candidate(
            "rejected",
            "project-memory-runtime",
            MemoryScope::Goal,
            Some("goal-memory-runtime"),
            None,
            MemoryCandidateState::Rejected,
        ),
        runtime_memory_candidate(
            "foreign-project",
            "project-other",
            MemoryScope::Project,
            None,
            None,
            MemoryCandidateState::New,
        ),
    ] {
        memory_store
            .append(&candidate)
            .expect("memory candidate appends");
    }

    let mut controller = ReadOnlyLoopController::start(
        EventLog::open(&event_path).expect("event log opens"),
        GoalContract::new(
            "goal-memory-runtime",
            "project-memory-runtime",
            "inspect repository",
            vec![".".into()],
            Vec::new(),
            vec!["completed".into()],
        ),
        "loop-memory-runtime",
        "task-run-memory-runtime",
        LoopBudget::new(2, 0, 0, 1, 0, 1, 1, 1),
    )
    .expect("loop starts");
    let query = MemoryQuery::new(
        "project-memory-runtime",
        Some("goal-memory-runtime"),
        Some("task-run-memory-runtime"),
    )
    .expect("memory query is valid");

    let retrieved = controller
        .retrieve_memory_candidates(&memory_store, &query, "memory-read-1")
        .expect("memory retrieval succeeds");

    assert_eq!(
        retrieved
            .iter()
            .map(|candidate| candidate.candidate_id.as_str())
            .collect::<Vec<_>>(),
        vec!["goal-visible", "project-visible", "task-visible"]
    );
    assert_eq!(controller.status(), LoopStatus::Running);
    let events = EventLog::open(&event_path)
        .expect("event log reopens")
        .read_all()
        .expect("events read");
    let retrieval_events = events
        .iter()
        .filter(|event| event.event_type.starts_with("memory_retrieval_"))
        .collect::<Vec<_>>();
    assert_eq!(
        retrieval_events
            .iter()
            .map(|event| event.event_type.as_str())
            .collect::<Vec<_>>(),
        vec![
            "memory_retrieval_requested",
            "memory_retrieval_filtered",
            "memory_retrieval_used"
        ]
    );
    assert_eq!(
        retrieval_events[0].payload["project_id"],
        "project-memory-runtime"
    );
    assert_eq!(retrieval_events[1].payload["candidate_count"], 3);
    assert_eq!(
        retrieval_events[1].payload["candidate_ids"],
        serde_json::json!(["goal-visible", "project-visible", "task-visible"])
    );
    assert_eq!(
        retrieval_events[2].payload["usage"],
        "returned_metadata_only"
    );
    assert!(retrieval_events.iter().all(|event| {
        let payload = event.payload.to_string();
        !payload.contains("synthetic-memory-secret") && !payload.contains("event:")
    }));
}

#[test]
fn controller_records_memory_candidate_with_metadata_only_event_lineage() {
    let directory = test_data_dir();
    fs::create_dir_all(&directory).expect("test directory creates");
    let event_path = directory.join("events.jsonl");
    let memory_store = MemoryStore::open(directory.join("memory.jsonl"));
    let mut controller = ReadOnlyLoopController::start(
        EventLog::open(&event_path).expect("event log opens"),
        GoalContract::new(
            "goal-memory-write",
            "project-memory-write",
            "record a candidate",
            vec![".".into()],
            Vec::new(),
            vec!["completed".into()],
        ),
        "loop-memory-write",
        "task-run-memory-write",
        LoopBudget::new(3, 0, 0, 1, 0, 1, 1, 1),
    )
    .expect("loop starts");
    controller
        .run_observation("source-step", "verified source observation")
        .expect("source event is recorded");
    let candidate = MemoryCandidate::new(
        "candidate-recorded",
        MemoryScope::Project,
        "project-memory-write",
        None,
        None,
        "candidate claim password=synthetic-memory-secret",
        MemoryProvenance {
            producer: "controller-test".into(),
            source_kind: "verification".into(),
        },
        vec!["source-step:completed".into()],
        MemoryConfidence::Medium,
        MemoryCandidateState::New,
    )
    .expect("candidate is valid");

    controller
        .record_memory_candidate(&memory_store, &candidate, "memory-write-1")
        .expect("candidate recording succeeds");

    let stored = memory_store.read_all().expect("memory store reads");
    assert_eq!(stored, vec![candidate.clone()]);
    let events = EventLog::open(&event_path)
        .expect("event log reopens")
        .read_all()
        .expect("events read");
    let written = events
        .iter()
        .find(|event| event.event_type == "memory_candidate_written")
        .expect("candidate write event exists");
    assert_eq!(written.payload["candidate_id"], "candidate-recorded");
    assert_eq!(written.payload["source_event_count"], 1);
    assert_eq!(written.payload["scope"], "project");
    let payload = written.payload.to_string();
    assert!(!payload.contains("candidate claim"));
    assert!(!payload.contains("synthetic-memory-secret"));
    assert!(!payload.contains("verified source observation"));
}

#[test]
fn controller_rejects_memory_candidate_scope_before_write_or_event() {
    let directory = test_data_dir();
    fs::create_dir_all(&directory).expect("test directory creates");
    let event_path = directory.join("events.jsonl");
    let memory_store = MemoryStore::open(directory.join("memory.jsonl"));
    let mut controller = ReadOnlyLoopController::start(
        EventLog::open(&event_path).expect("event log opens"),
        GoalContract::new(
            "goal-memory-scope-write",
            "project-memory-scope-write",
            "record a candidate",
            vec![".".into()],
            Vec::new(),
            vec!["completed".into()],
        ),
        "loop-memory-scope-write",
        "task-run-memory-scope-write",
        LoopBudget::new(2, 0, 0, 1, 0, 1, 1, 1),
    )
    .expect("loop starts");
    let candidate = runtime_memory_candidate(
        "foreign-candidate",
        "foreign-project",
        MemoryScope::Project,
        None,
        None,
        MemoryCandidateState::New,
    );

    assert!(matches!(
        controller.record_memory_candidate(&memory_store, &candidate, "memory-write-scope"),
        Err(LoopControllerError::MemoryCandidateScopeMismatch)
    ));
    assert!(memory_store
        .read_all()
        .expect("memory store reads")
        .is_empty());
    let events = EventLog::open(&event_path)
        .expect("event log reopens")
        .read_all()
        .expect("events read");
    assert!(events
        .iter()
        .all(|event| !event.event_type.starts_with("memory_candidate_write_")));
}

#[test]
fn controller_records_duplicate_memory_candidate_failure_without_success_event() {
    let directory = test_data_dir();
    fs::create_dir_all(&directory).expect("test directory creates");
    let event_path = directory.join("events.jsonl");
    let memory_store = MemoryStore::open(directory.join("memory.jsonl"));
    let mut controller = ReadOnlyLoopController::start(
        EventLog::open(&event_path).expect("event log opens"),
        GoalContract::new(
            "goal-memory-duplicate",
            "project-memory-duplicate",
            "record a candidate",
            vec![".".into()],
            Vec::new(),
            vec!["completed".into()],
        ),
        "loop-memory-duplicate",
        "task-run-memory-duplicate",
        LoopBudget::new(4, 0, 0, 1, 0, 1, 1, 1),
    )
    .expect("loop starts");
    controller
        .run_observation("source-step", "verified source observation")
        .expect("source event is recorded");
    let candidate = MemoryCandidate::new(
        "candidate-duplicate",
        MemoryScope::Project,
        "project-memory-duplicate",
        None,
        None,
        "candidate claim",
        MemoryProvenance {
            producer: "controller-test".into(),
            source_kind: "verification".into(),
        },
        vec!["source-step:completed".into()],
        MemoryConfidence::Medium,
        MemoryCandidateState::New,
    )
    .expect("candidate is valid");

    controller
        .record_memory_candidate(&memory_store, &candidate, "memory-write-first")
        .expect("first candidate recording succeeds");
    let error = controller
        .record_memory_candidate(&memory_store, &candidate, "memory-write-duplicate")
        .expect_err("duplicate candidate is rejected");
    assert!(matches!(error, LoopControllerError::MemoryStore(_)));

    let events = EventLog::open(&event_path)
        .expect("event log reopens")
        .read_all()
        .expect("events read");
    assert!(events.iter().any(|event| {
        event.event_type == "memory_candidate_write_failed"
            && event.payload["failure_class"] == "duplicate_candidate"
    }));
    assert_eq!(
        events
            .iter()
            .filter(|event| event.event_type == "memory_candidate_written")
            .count(),
        1
    );
}

#[test]
fn controller_records_promotion_decision_without_activating_candidate() {
    let directory = test_data_dir();
    fs::create_dir_all(&directory).expect("test directory creates");
    let event_path = directory.join("events.jsonl");
    let memory_store = MemoryStore::open(directory.join("memory.jsonl"));
    let mut controller = ReadOnlyLoopController::start(
        EventLog::open(&event_path).expect("event log opens"),
        GoalContract::new(
            "goal-memory-promotion",
            "project-memory-promotion",
            "record a promotion decision",
            vec![".".into()],
            Vec::new(),
            vec!["completed".into()],
        ),
        "loop-memory-promotion",
        "task-run-memory-promotion",
        LoopBudget::new(3, 0, 0, 1, 0, 1, 1, 1),
    )
    .expect("loop starts");
    controller
        .run_observation("source-step", "verified source observation")
        .expect("source event is recorded");
    let candidate = MemoryCandidate::new(
        "candidate-promotion",
        MemoryScope::Project,
        "project-memory-promotion",
        None,
        None,
        "candidate claim password=synthetic-memory-secret",
        MemoryProvenance {
            producer: "controller-test".into(),
            source_kind: "verification".into(),
        },
        vec!["source-step:completed".into()],
        MemoryConfidence::High,
        MemoryCandidateState::New,
    )
    .expect("candidate is valid");
    controller
        .record_memory_candidate(&memory_store, &candidate, "memory-write-promotion")
        .expect("candidate recording succeeds");
    let before = memory_store.read_all().expect("memory store reads");

    controller
        .record_memory_promotion_decision(
            &candidate,
            "promotion-1",
            MemoryScope::Project,
            MemoryScope::Project,
            MemoryPromotionDecision::Accept,
            "validated password=synthetic-memory-secret",
            &["source-step:completed".into()],
            "policy-decision-1",
            Some("human-review-1"),
        )
        .expect("promotion decision audit succeeds");

    assert_eq!(
        memory_store.read_all().expect("memory store rereads"),
        before
    );
    let events = EventLog::open(&event_path)
        .expect("event log reopens")
        .read_all()
        .expect("events read");
    let decision = events
        .iter()
        .find(|event| event.event_type == "memory_promotion_decision_recorded")
        .expect("promotion decision event exists");
    assert_eq!(decision.payload["decision"], "accept");
    assert_eq!(decision.payload["materialized"], false);
    assert_eq!(decision.payload["validation_ref_count"], 1);
    let payload = decision.payload.to_string();
    assert!(!payload.contains("synthetic-memory-secret"));
    assert!(!payload.contains("validated password"));
}

#[test]
fn controller_rejects_memory_query_outside_current_loop_scope_without_event() {
    let directory = test_data_dir();
    fs::create_dir_all(&directory).expect("test directory creates");
    let event_path = directory.join("events.jsonl");
    let memory_store = MemoryStore::open(directory.join("memory.jsonl"));
    memory_store
        .append(&runtime_memory_candidate(
            "foreign-project",
            "project-other",
            MemoryScope::Project,
            None,
            None,
            MemoryCandidateState::New,
        ))
        .expect("foreign memory candidate appends");
    let mut controller = ReadOnlyLoopController::start(
        EventLog::open(&event_path).expect("event log opens"),
        GoalContract::new(
            "goal-memory-scope",
            "project-memory-scope",
            "inspect repository",
            vec![".".into()],
            Vec::new(),
            vec!["completed".into()],
        ),
        "loop-memory-scope",
        "task-run-memory-scope",
        LoopBudget::new(2, 0, 0, 1, 0, 1, 1, 1),
    )
    .expect("loop starts");
    let query = MemoryQuery::new("project-other", None, None).expect("memory query is valid");

    assert!(matches!(
        controller.retrieve_memory_candidates(&memory_store, &query, "memory-read-foreign"),
        Err(LoopControllerError::MemoryQueryScopeMismatch)
    ));
    let events = EventLog::open(&event_path)
        .expect("event log reopens")
        .read_all()
        .expect("events read");
    assert!(!events
        .iter()
        .any(|event| event.event_type.starts_with("memory_retrieval_")));
}

#[test]
fn controller_fails_closed_when_memory_retrieval_audit_cannot_be_written() {
    let directory = test_data_dir();
    fs::create_dir_all(&directory).expect("test directory creates");
    let event_path = directory.join("events.jsonl");
    let memory_store = MemoryStore::open(directory.join("memory.jsonl"));
    memory_store
        .append(&runtime_memory_candidate(
            "candidate-audit-failure",
            "project-memory-audit-failure",
            MemoryScope::Project,
            None,
            None,
            MemoryCandidateState::New,
        ))
        .expect("memory candidate appends");
    let mut controller = ReadOnlyLoopController::start(
        EventLog::open(&event_path).expect("event log opens"),
        GoalContract::new(
            "goal-memory-audit-failure",
            "project-memory-audit-failure",
            "inspect repository",
            vec![".".into()],
            Vec::new(),
            vec!["completed".into()],
        ),
        "loop-memory-audit-failure",
        "task-run-memory-audit-failure",
        LoopBudget::new(2, 0, 0, 1, 0, 1, 1, 1),
    )
    .expect("loop starts");
    fs::remove_file(&event_path).expect("event file removes");
    fs::create_dir(&event_path).expect("event path becomes directory");
    let query = MemoryQuery::new("project-memory-audit-failure", None, None)
        .expect("memory query is valid");

    assert!(matches!(
        controller.retrieve_memory_candidates(&memory_store, &query, "memory-read-failure"),
        Err(LoopControllerError::EventLog(_))
    ));
}

#[test]
fn controller_rejects_memory_retrieval_when_paused_or_terminal() {
    let paused_directory = test_data_dir();
    fs::create_dir_all(&paused_directory).expect("paused test directory creates");
    let paused_event_path = paused_directory.join("events.jsonl");
    let paused_memory_store = MemoryStore::open(paused_directory.join("memory.jsonl"));
    paused_memory_store
        .append(&runtime_memory_candidate(
            "paused-candidate",
            "project-memory-lifecycle",
            MemoryScope::Project,
            None,
            None,
            MemoryCandidateState::New,
        ))
        .expect("paused memory candidate appends");
    let paused_inbox = UserCommandInbox::open(paused_directory.join("commands.jsonl"));
    paused_inbox
        .submit(steering_command(
            "pause-memory-retrieval",
            "project-memory-lifecycle",
            "goal-memory-lifecycle",
            "task-run-memory-lifecycle",
            "loop-memory-lifecycle",
            CommandKind::Pause,
        ))
        .expect("pause command submits");
    let mut paused_controller = ReadOnlyLoopController::start(
        EventLog::open(&paused_event_path).expect("paused event log opens"),
        GoalContract::new(
            "goal-memory-lifecycle",
            "project-memory-lifecycle",
            "inspect repository",
            vec![".".into()],
            Vec::new(),
            vec!["completed".into()],
        ),
        "loop-memory-lifecycle",
        "task-run-memory-lifecycle",
        LoopBudget::new(2, 0, 0, 1, 0, 1, 1, 1),
    )
    .expect("paused loop starts")
    .with_command_inbox(paused_inbox);
    let lifecycle_query = MemoryQuery::new(
        "project-memory-lifecycle",
        Some("goal-memory-lifecycle"),
        Some("task-run-memory-lifecycle"),
    )
    .expect("lifecycle query is valid");
    assert!(matches!(
        paused_controller.retrieve_memory_candidates(
            &paused_memory_store,
            &lifecycle_query,
            "memory-read-paused"
        ),
        Err(LoopControllerError::Paused)
    ));
    assert_eq!(paused_controller.status(), LoopStatus::Paused);
    let paused_events = EventLog::open(paused_event_path)
        .expect("paused event log reopens")
        .read_all()
        .expect("paused events read");
    assert!(!paused_events
        .iter()
        .any(|event| event.event_type.starts_with("memory_retrieval_")));

    let terminal_directory = test_data_dir();
    fs::create_dir_all(&terminal_directory).expect("terminal test directory creates");
    let terminal_event_path = terminal_directory.join("events.jsonl");
    let terminal_memory_store = MemoryStore::open(terminal_directory.join("memory.jsonl"));
    terminal_memory_store
        .append(&runtime_memory_candidate(
            "terminal-candidate",
            "project-memory-lifecycle-terminal",
            MemoryScope::Project,
            None,
            None,
            MemoryCandidateState::New,
        ))
        .expect("terminal memory candidate appends");
    let mut terminal_controller = ReadOnlyLoopController::start(
        EventLog::open(&terminal_event_path).expect("terminal event log opens"),
        GoalContract::new(
            "goal-memory-lifecycle-terminal",
            "project-memory-lifecycle-terminal",
            "inspect repository",
            vec![".".into()],
            Vec::new(),
            vec!["completed".into()],
        ),
        "loop-memory-lifecycle-terminal",
        "task-run-memory-lifecycle-terminal",
        LoopBudget::new(2, 0, 0, 1, 0, 1, 1, 1),
    )
    .expect("terminal loop starts");
    terminal_controller.complete().expect("loop completes");
    let terminal_query = MemoryQuery::new("project-memory-lifecycle-terminal", None, None)
        .expect("terminal query is valid");
    assert!(matches!(
        terminal_controller.retrieve_memory_candidates(
            &terminal_memory_store,
            &terminal_query,
            "memory-read-terminal"
        ),
        Err(LoopControllerError::Terminal)
    ));
    let terminal_events = EventLog::open(terminal_event_path)
        .expect("terminal event log reopens")
        .read_all()
        .expect("terminal events read");
    assert!(!terminal_events
        .iter()
        .any(|event| event.event_type.starts_with("memory_retrieval_")));
}

#[test]
fn observation_event_redacts_sensitive_text_before_event_log_persistence() {
    let directory = test_data_dir();
    fs::create_dir_all(&directory).expect("test directory creates");
    let event_path = directory.join("events.jsonl");
    let mut controller = ReadOnlyLoopController::start(
        EventLog::open(&event_path).expect("event log opens"),
        GoalContract::new(
            "goal-observation-redaction",
            "project-observation-redaction",
            "inspect repository",
            vec![".".into()],
            Vec::new(),
            vec!["completed".into()],
        ),
        "loop-observation-redaction",
        "task-run-observation-redaction",
        LoopBudget::new(2, 0, 0, 1, 0, 1, 1, 1),
    )
    .expect("loop starts");

    controller
        .run_observation(
            "step-observation-redaction",
            "password=synthetic-password at https://alice:synthetic-url-secret@example.test",
        )
        .expect("observation succeeds");

    let persisted = fs::read_to_string(event_path).expect("event log reads");
    assert!(!persisted.contains("synthetic-password"));
    assert!(!persisted.contains("synthetic-url-secret"));
    assert!(persisted.contains("[REDACTED:PASSWORD]"));
    assert!(persisted.contains("[REDACTED:URL_PASSWORD]"));
    let _ = fs::remove_dir_all(directory);
}

#[test]
fn controller_executes_model_action_through_policy_and_verification() {
    let directory = test_data_dir();
    fs::create_dir_all(&directory).expect("test directory creates");
    let workspace = std::env::current_dir().expect("workspace exists");
    let root = workspace.display().to_string();
    let mut controller = ReadOnlyLoopController::start(
        EventLog::open(directory.join("events.jsonl")).expect("event log opens"),
        GoalContract::new(
            "goal-action",
            "project-action",
            "inspect repository",
            vec!["git_status".into(), "markdown_link_check".into()],
            Vec::new(),
            vec!["completed".into()],
        ),
        "loop-action",
        "task-run-action",
        LoopBudget::new(2, 0, 0, 1, 0, 4, 1, 1),
    )
    .expect("loop starts");

    let action = ModelActionEnvelope {
        schema_version: steerbox::model_contracts::MODEL_ACTION_SCHEMA_VERSION,
        action_id: "action-read-status".into(),
        tool_id: "git_status".into(),
        parameters: serde_json::json!({"scope": "workspace"}),
        path_scope: ActionPathScope {
            root,
            allowed_paths: vec!["README.md".into()],
        },
        verification: VerificationContract {
            verifier_tool_id: "markdown_link_check".into(),
            parameters: serde_json::json!({"path": "README.md"}),
        },
    };

    let execution = controller
        .execute_model_action(
            &ToolRegistry::baseline().expect("baseline tools register"),
            &PolicyGate::new(),
            &ReadOnlyToolExecutor,
            &workspace,
            &action,
            "step-action",
        )
        .expect("model action executes and verifies");
    assert_eq!(execution.action_id, "action-read-status");
    assert!(execution.action_output.result["executed"]
        .as_bool()
        .unwrap());
    assert!(execution.verification_output.result["executed"]
        .as_bool()
        .unwrap());

    let events = EventLog::open(directory.join("events.jsonl"))
        .expect("event log reopens")
        .read_all()
        .expect("events read");
    assert!(events
        .iter()
        .any(|event| event.event_type == "model_action_started"));
    assert!(events
        .iter()
        .any(|event| event.event_type == "model_action_verified"));
}

fn workspace_edit_action(workspace: &std::path::Path) -> ModelActionEnvelope {
    ModelActionEnvelope {
        schema_version: steerbox::model_contracts::MODEL_ACTION_SCHEMA_VERSION,
        action_id: "action-edit-markdown".into(),
        tool_id: "edit_markdown".into(),
        parameters: serde_json::json!({
            "path": "README.md",
            "expected": "before",
            "replacement": "after"
        }),
        path_scope: ActionPathScope {
            root: workspace.display().to_string(),
            allowed_paths: vec!["README.md".into()],
        },
        verification: VerificationContract {
            verifier_tool_id: "markdown_link_check".into(),
            parameters: serde_json::json!({"path": "README.md"}),
        },
    }
}

#[test]
fn controller_executes_approved_workspace_edit_and_verifies_it() {
    let directory = test_data_dir();
    fs::create_dir_all(&directory).expect("test directory creates");
    fs::write(directory.join("README.md"), "before").expect("markdown fixture writes");
    let registry = ToolRegistry::baseline().expect("baseline tools register");
    let gate = PolicyGate::new();
    let action = workspace_edit_action(&directory);
    let mut controller = ReadOnlyLoopController::start(
        EventLog::open(directory.join("events.jsonl")).expect("event log opens"),
        GoalContract::new(
            "goal-write",
            "project-write",
            "edit markdown",
            vec![".".into()],
            Vec::new(),
            vec!["completed".into()],
        ),
        "loop-write",
        "task-run-write",
        LoopBudget::new(3, 0, 0, 1, 0, 4, 1, 1),
    )
    .expect("loop starts")
    .with_checkpoint_store(directory.join("checkpoints.jsonl"))
    .with_audit_stores(
        directory.join("traces.jsonl"),
        directory.join("side-effects.jsonl"),
    );

    controller
        .execute_workspace_edit_action(
            &registry,
            &gate,
            &WorkspaceToolExecutor,
            &ReadOnlyToolExecutor,
            &directory,
            &action,
            "step-write",
            true,
        )
        .expect("approved workspace edit succeeds");

    assert_eq!(
        fs::read_to_string(directory.join("README.md")).unwrap(),
        "after"
    );
    let events = EventLog::open(directory.join("events.jsonl"))
        .expect("event log reopens")
        .read_all()
        .expect("events read");
    assert!(events
        .iter()
        .any(|event| event.event_type == "model_action_started"));
    assert!(events
        .iter()
        .any(|event| event.event_type == "file_change_recorded"));
    assert!(events
        .iter()
        .any(|event| event.event_type == "model_action_verified"));
    assert!(events
        .iter()
        .filter(|event| {
            matches!(
                event.event_type.as_str(),
                "model_action_started" | "file_change_recorded" | "model_action_verified"
            )
        })
        .all(|event| {
            let payload = event.payload.to_string();
            !payload.contains("before") && !payload.contains("after")
        }));
    let checkpoints = CheckpointStore::open(directory.join("checkpoints.jsonl"))
        .read_all()
        .expect("checkpoints read");
    assert_eq!(checkpoints.len(), 2);
    assert_eq!(checkpoints[0].files, vec!["README.md"]);
    assert!(checkpoints[0].diff_boundary.starts_with("before=sha256:"));
    assert!(checkpoints[0].diff_boundary.ends_with(";after=pending"));
    assert!(checkpoints[1].diff_boundary.starts_with("before=sha256:"));
    assert!(checkpoints[1].diff_boundary.contains(";after=sha256:"));
    assert_eq!(
        checkpoints[1].recoverability,
        "verified_local_workspace_restore_or_escalate"
    );
    let side_effects = SideEffectLedger::open(directory.join("side-effects.jsonl"))
        .read_all()
        .expect("side effects read");
    assert!(side_effects.iter().any(|record| record.status == "started"));
    assert!(side_effects
        .iter()
        .any(|record| record.status == "completed"));
    let _ = fs::remove_dir_all(directory);
}

#[test]
fn controller_rejects_semantically_failed_markdown_verification() {
    let directory = test_data_dir();
    fs::create_dir_all(&directory).expect("test directory creates");
    fs::write(directory.join("README.md"), "before").expect("markdown fixture writes");
    let registry = ToolRegistry::baseline().expect("baseline tools register");
    let gate = PolicyGate::new();
    let mut action = workspace_edit_action(&directory);
    action.parameters["replacement"] = serde_json::json!("after\n[broken](missing.md)");
    let mut controller = ReadOnlyLoopController::start(
        EventLog::open(directory.join("events.jsonl")).expect("event log opens"),
        GoalContract::new(
            "goal-write-verify-fail",
            "project-write-verify-fail",
            "edit markdown",
            vec![".".into()],
            Vec::new(),
            vec!["completed".into()],
        ),
        "loop-write-verify-fail",
        "task-run-write-verify-fail",
        LoopBudget::new(3, 0, 0, 1, 0, 4, 1, 1),
    )
    .expect("loop starts")
    .with_checkpoint_store(directory.join("checkpoints.jsonl"))
    .with_audit_stores(
        directory.join("traces.jsonl"),
        directory.join("side-effects.jsonl"),
    );

    assert!(matches!(
        controller.execute_workspace_edit_action(
            &registry,
            &gate,
            &WorkspaceToolExecutor,
            &ReadOnlyToolExecutor,
            &directory,
            &action,
            "step-write-verify-fail",
            true,
        ),
        Err(LoopControllerError::WorkspaceToolExecution(_))
    ));
    assert_eq!(
        fs::read_to_string(directory.join("README.md")).unwrap(),
        "after\n[broken](missing.md)"
    );
    let events = EventLog::open(directory.join("events.jsonl"))
        .expect("event log reopens")
        .read_all()
        .expect("events read");
    assert!(events
        .iter()
        .any(|event| event.event_type == "model_action_failed"));
    assert!(!events
        .iter()
        .any(|event| event.event_type == "model_action_verified"));
    let checkpoints = CheckpointStore::open(directory.join("checkpoints.jsonl"))
        .read_all()
        .expect("checkpoints read");
    assert_eq!(checkpoints.len(), 2);
    assert!(checkpoints[1].diff_boundary.contains(";after=sha256:"));
    assert_eq!(
        checkpoints[1].recoverability,
        "verification_failed_local_workspace_restore_or_escalate"
    );
    let _ = fs::remove_dir_all(directory);
}

#[test]
fn controller_executes_bounded_workspace_edit_in_autonomous_mode_without_prompt() {
    let directory = test_data_dir();
    fs::create_dir_all(&directory).expect("test directory creates");
    fs::write(directory.join("README.md"), "before").expect("markdown fixture writes");
    let registry = ToolRegistry::baseline().expect("baseline tools register");
    let gate = PolicyGate::new();
    let action = workspace_edit_action(&directory);
    let mut goal = GoalContract::new(
        "goal-auto-write",
        "project-auto-write",
        "edit bounded markdown automatically",
        vec![".".into()],
        Vec::new(),
        vec!["completed".into()],
    );
    goal.allowed_tools = vec!["edit_markdown".into(), "markdown_link_check".into()];
    goal.human_steering_mode = HumanSteeringMode::AutonomousLoop;
    let mut controller = ReadOnlyLoopController::start(
        EventLog::open(directory.join("events.jsonl")).expect("event log opens"),
        goal,
        "loop-auto-write",
        "task-run-auto-write",
        LoopBudget::new(3, 0, 0, 1, 0, 4, 1, 1),
    )
    .expect("loop starts")
    .with_checkpoint_store(directory.join("checkpoints.jsonl"))
    .with_audit_stores(
        directory.join("traces.jsonl"),
        directory.join("side-effects.jsonl"),
    );

    controller
        .execute_workspace_edit_action(
            &registry,
            &gate,
            &WorkspaceToolExecutor,
            &ReadOnlyToolExecutor,
            &directory,
            &action,
            "step-auto-write",
            false,
        )
        .expect("autonomous bounded workspace edit succeeds");

    assert_eq!(
        fs::read_to_string(directory.join("README.md")).unwrap(),
        "after"
    );
    let events = EventLog::open(directory.join("events.jsonl"))
        .expect("event log reopens")
        .read_all()
        .expect("events read");
    assert!(events.iter().any(|event| {
        event.event_type == "policy_decision" && event.payload["decision"] == "allow"
    }));
    assert!(events
        .iter()
        .any(|event| event.event_type == "model_action_verified"));
    let _ = fs::remove_dir_all(directory);
}

#[test]
fn controller_rejects_unapproved_workspace_edit_without_writing_or_verifying() {
    let directory = test_data_dir();
    fs::create_dir_all(&directory).expect("test directory creates");
    fs::write(directory.join("README.md"), "before").expect("markdown fixture writes");
    let registry = ToolRegistry::baseline().expect("baseline tools register");
    let gate = PolicyGate::new();
    let action = workspace_edit_action(&directory);
    let mut controller = ReadOnlyLoopController::start(
        EventLog::open(directory.join("events.jsonl")).expect("event log opens"),
        GoalContract::new(
            "goal-write-denied",
            "project-write-denied",
            "edit markdown",
            vec![".".into()],
            Vec::new(),
            vec!["completed".into()],
        ),
        "loop-write-denied",
        "task-run-write-denied",
        LoopBudget::new(2, 0, 0, 1, 0, 2, 1, 1),
    )
    .expect("loop starts");

    assert!(matches!(
        controller.execute_workspace_edit_action(
            &registry,
            &gate,
            &WorkspaceToolExecutor,
            &ReadOnlyToolExecutor,
            &directory,
            &action,
            "step-write-denied",
            false,
        ),
        Err(LoopControllerError::PolicyDenied { .. })
    ));
    assert_eq!(
        fs::read_to_string(directory.join("README.md")).unwrap(),
        "before"
    );
    let events = EventLog::open(directory.join("events.jsonl"))
        .expect("event log reopens")
        .read_all()
        .expect("events read");
    assert!(events
        .iter()
        .any(|event| event.event_type == "model_action_failed"));
    assert!(!events
        .iter()
        .any(|event| event.event_type == "model_action_verified"));
    let _ = fs::remove_dir_all(directory);
}

#[test]
fn model_step_routes_provider_action_through_policy_and_verification() {
    let directory = test_data_dir();
    fs::create_dir_all(&directory).expect("test directory creates");
    let workspace = std::env::current_dir().expect("workspace exists");
    let action = ModelActionEnvelope {
        schema_version: steerbox::model_contracts::MODEL_ACTION_SCHEMA_VERSION,
        action_id: "action-model-step".into(),
        tool_id: "git_status".into(),
        parameters: serde_json::json!({"scope": "workspace"}),
        path_scope: ActionPathScope {
            root: workspace.display().to_string(),
            allowed_paths: vec!["README.md".into()],
        },
        verification: VerificationContract {
            verifier_tool_id: "markdown_link_check".into(),
            parameters: serde_json::json!({"path": "README.md"}),
        },
    };
    let mut controller = ReadOnlyLoopController::start(
        EventLog::open(directory.join("events.jsonl")).expect("event log opens"),
        GoalContract::new(
            "goal-model-action",
            "project-model-action",
            "inspect repository",
            vec!["git_status".into(), "markdown_link_check".into()],
            Vec::new(),
            vec!["completed".into()],
        ),
        "loop-model-action",
        "task-run-model-action",
        LoopBudget::new(2, 0, 1, 1, 0, 4, 1, 1),
    )
    .expect("loop starts");
    let adapter = ActionFixtureAdapter { action };
    let registry = ToolRegistry::baseline().expect("baseline tools register");
    let gate = PolicyGate::new();
    let executor = ReadOnlyToolExecutor;
    let action_runtime = ModelActionRuntime::new(&registry, &gate, &executor, &workspace);

    let decision = controller
        .run_model_step_with_action(
            &adapter,
            &action_runtime,
            "fixture-action-model",
            "step-model-action",
            "repository observed",
        )
        .expect("model action step executes");
    assert_eq!(decision, ModelDecision::ContinueObservation);

    let events = EventLog::open(directory.join("events.jsonl"))
        .expect("event log reopens")
        .read_all()
        .expect("events read");
    assert!(events
        .iter()
        .any(|event| event.event_type == "model_action_started"));
    assert_eq!(
        events
            .iter()
            .filter(|event| event.event_type == "tool_call_completed")
            .count(),
        2
    );
    assert!(events
        .iter()
        .any(|event| event.event_type == "model_action_verified"));
    assert!(events
        .iter()
        .any(|event| event.event_type == "step_completed"));
}

#[test]
fn next_model_step_receives_only_redacted_tool_output() {
    let directory = test_data_dir();
    fs::create_dir_all(&directory).expect("event directory creates");
    let workspace = tempfile::tempdir().expect("workspace creates");
    let mut controller = ReadOnlyLoopController::start(
        EventLog::open(directory.join("events.jsonl")).expect("event log opens"),
        GoalContract::new(
            "goal-tool-context",
            "project-tool-context",
            "inspect repository",
            vec![".".into()],
            Vec::new(),
            vec!["completed".into()],
        ),
        "loop-tool-context",
        "task-run-tool-context",
        LoopBudget::new(2, 0, 1, 1, 0, 2, 1, 1),
    )
    .expect("loop starts");
    let registry = ToolRegistry::baseline().expect("baseline tools register");
    let gate = PolicyGate::new();
    let executor = ReadOnlyToolExecutor;
    let runtime = ModelActionRuntime::new(&registry, &gate, &executor, workspace.path());
    let observations = Arc::new(Mutex::new(Vec::new()));
    let adapter = RecordingObservationAdapter {
        observations: observations.clone(),
    };
    let output = steerbox::tool_executor::ToolCallOutput {
        summary: "diff contains password=synthetic-tool-password".into(),
        result: serde_json::json!({
            "stdout": "Authorization: Bearer synthetic-tool-token",
            "safe": "tracked change"
        }),
    };

    controller
        .run_model_step_with_tool_output(
            &adapter,
            &runtime,
            "fixture-context-model",
            "step-tool-context",
            &output,
        )
        .expect("model step accepts redacted tool output");

    let observation = observations
        .lock()
        .expect("observation lock")
        .first()
        .cloned()
        .expect("one model observation");
    assert!(!observation.contains("synthetic-tool-password"));
    assert!(!observation.contains("synthetic-tool-token"));
    assert!(observation.contains("tracked change"));
}

#[test]
fn oversized_tool_output_stops_before_model_adapter_call() {
    let directory = test_data_dir();
    fs::create_dir_all(&directory).expect("event directory creates");
    let workspace = tempfile::tempdir().expect("workspace creates");
    let mut controller = ReadOnlyLoopController::start(
        EventLog::open(directory.join("events.jsonl")).expect("event log opens"),
        GoalContract::new(
            "goal-tool-context-limit",
            "project-tool-context-limit",
            "inspect repository",
            vec![".".into()],
            Vec::new(),
            vec!["completed".into()],
        ),
        "loop-tool-context-limit",
        "task-run-tool-context-limit",
        LoopBudget::new(2, 0, 1, 1, 0, 2, 1, 1),
    )
    .expect("loop starts");
    let registry = ToolRegistry::baseline().expect("baseline tools register");
    let gate = PolicyGate::new();
    let executor = ReadOnlyToolExecutor;
    let runtime = ModelActionRuntime::new(&registry, &gate, &executor, workspace.path());
    let observations = Arc::new(Mutex::new(Vec::new()));
    let adapter = RecordingObservationAdapter {
        observations: observations.clone(),
    };
    let output = steerbox::tool_executor::ToolCallOutput {
        summary: "x".repeat(steerbox::redaction::DEFAULT_MAX_INPUT_BYTES + 1),
        result: serde_json::json!({"safe": "metadata"}),
    };

    assert!(matches!(
        controller.run_model_step_with_tool_output(
            &adapter,
            &runtime,
            "fixture-context-model",
            "step-tool-context-limit",
            &output,
        ),
        Err(LoopControllerError::ModelToolContextRedaction(_))
    ));
    assert!(observations.lock().expect("observation lock").is_empty());
}

#[test]
fn next_model_step_redacts_action_and_verification_outputs() {
    let directory = test_data_dir();
    fs::create_dir_all(&directory).expect("event directory creates");
    let workspace = tempfile::tempdir().expect("workspace creates");
    let mut controller = ReadOnlyLoopController::start(
        EventLog::open(directory.join("events.jsonl")).expect("event log opens"),
        GoalContract::new(
            "goal-tool-results",
            "project-tool-results",
            "inspect repository",
            vec![".".into()],
            Vec::new(),
            vec!["completed".into()],
        ),
        "loop-tool-results",
        "task-run-tool-results",
        LoopBudget::new(2, 0, 1, 1, 0, 2, 1, 1),
    )
    .expect("loop starts");
    let registry = ToolRegistry::baseline().expect("baseline tools register");
    let gate = PolicyGate::new();
    let executor = ReadOnlyToolExecutor;
    let runtime = ModelActionRuntime::new(&registry, &gate, &executor, workspace.path());
    let observations = Arc::new(Mutex::new(Vec::new()));
    let adapter = RecordingObservationAdapter {
        observations: observations.clone(),
    };
    let action_output = steerbox::tool_executor::ToolCallOutput {
        summary: "action changed tracked file; password=synthetic-action-password".into(),
        result: serde_json::json!({
            "stdout": "Authorization: Bearer synthetic-action-token",
            "safe_action": "file updated"
        }),
    };
    let verification_output = steerbox::tool_executor::ToolCallOutput {
        summary: "verification passed; token=synthetic-verification-token".into(),
        result: serde_json::json!({
            "stdout": "password=synthetic-verification-password",
            "safe_verification": "hash matches"
        }),
    };

    controller
        .run_model_step_with_tool_results(
            &adapter,
            &runtime,
            "fixture-context-model",
            "step-tool-results",
            "action-1",
            "file_write",
            &action_output,
            &verification_output,
        )
        .expect("model step accepts redacted action and verification outputs");

    let observation = observations
        .lock()
        .expect("observation lock")
        .first()
        .cloned()
        .expect("one model observation");
    assert!(observation.contains("action-1"));
    assert!(observation.contains("file_write"));
    assert!(observation.contains("file updated"));
    assert!(observation.contains("hash matches"));
    for secret in [
        "synthetic-action-password",
        "synthetic-action-token",
        "synthetic-verification-token",
        "synthetic-verification-password",
    ] {
        assert!(!observation.contains(secret), "secret leaked: {secret}");
    }
    assert!(action_output.summary.contains("synthetic-action-password"));
    assert!(verification_output
        .summary
        .contains("synthetic-verification-token"));
}

#[test]
fn combined_tool_outputs_stop_before_model_adapter_call() {
    let directory = test_data_dir();
    fs::create_dir_all(&directory).expect("event directory creates");
    let workspace = tempfile::tempdir().expect("workspace creates");
    let mut controller = ReadOnlyLoopController::start(
        EventLog::open(directory.join("events.jsonl")).expect("event log opens"),
        GoalContract::new(
            "goal-tool-results-limit",
            "project-tool-results-limit",
            "inspect repository",
            vec![".".into()],
            Vec::new(),
            vec!["completed".into()],
        ),
        "loop-tool-results-limit",
        "task-run-tool-results-limit",
        LoopBudget::new(2, 0, 1, 1, 0, 2, 1, 1),
    )
    .expect("loop starts");
    let registry = ToolRegistry::baseline().expect("baseline tools register");
    let gate = PolicyGate::new();
    let executor = ReadOnlyToolExecutor;
    let runtime = ModelActionRuntime::new(&registry, &gate, &executor, workspace.path());
    let observations = Arc::new(Mutex::new(Vec::new()));
    let adapter = RecordingObservationAdapter {
        observations: observations.clone(),
    };
    let action_output = steerbox::tool_executor::ToolCallOutput {
        summary: "a".repeat(40 * 1024),
        result: serde_json::json!({"safe": "action"}),
    };
    let verification_output = steerbox::tool_executor::ToolCallOutput {
        summary: "v".repeat(40 * 1024),
        result: serde_json::json!({"safe": "verification"}),
    };

    assert!(matches!(
        controller.run_model_step_with_tool_results(
            &adapter,
            &runtime,
            "fixture-context-model",
            "step-tool-results-limit",
            "action-1",
            "file_write",
            &action_output,
            &verification_output,
        ),
        Err(LoopControllerError::ModelToolContextTooLarge { .. })
    ));
    assert!(observations.lock().expect("observation lock").is_empty());
}

#[test]
fn controller_rejects_free_shell_model_action_before_policy_or_execution() {
    let directory = test_data_dir();
    fs::create_dir_all(&directory).expect("test directory creates");
    let workspace = std::env::current_dir().expect("workspace exists");
    let action = ModelActionEnvelope {
        schema_version: steerbox::model_contracts::MODEL_ACTION_SCHEMA_VERSION,
        action_id: "action-shell".into(),
        tool_id: "rg_fixed".into(),
        parameters: serde_json::json!({"command": "git status", "path": "README.md"}),
        path_scope: ActionPathScope {
            root: workspace.display().to_string(),
            allowed_paths: vec!["README.md".into()],
        },
        verification: VerificationContract {
            verifier_tool_id: "markdown_link_check".into(),
            parameters: serde_json::json!({"path": "README.md"}),
        },
    };
    let mut controller = ReadOnlyLoopController::start(
        EventLog::open(directory.join("events.jsonl")).expect("event log opens"),
        GoalContract::new(
            "goal-shell",
            "project-shell",
            "inspect repository",
            vec!["rg_fixed".into()],
            Vec::new(),
            vec!["completed".into()],
        ),
        "loop-shell",
        "task-run-shell",
        LoopBudget::new(2, 0, 0, 1, 0, 2, 1, 1),
    )
    .expect("loop starts");

    assert!(matches!(
        controller.execute_model_action(
            &ToolRegistry::baseline().expect("baseline tools register"),
            &PolicyGate::new(),
            &ReadOnlyToolExecutor,
            &workspace,
            &action,
            "step-shell",
        ),
        Err(LoopControllerError::ModelContract(_))
    ));
}

#[test]
fn controller_binds_initial_attempt_and_finishes_task_run_without_resetting_budget() {
    let directory = test_data_dir();
    fs::create_dir_all(&directory).expect("test directory creates");
    let mut controller = ReadOnlyLoopController::start(
        EventLog::open(directory.join("events.jsonl")).expect("event log opens"),
        GoalContract::new(
            "goal-attempt",
            "project-attempt",
            "inspect repository",
            vec![".".into()],
            Vec::new(),
            vec!["completed".into()],
        ),
        "loop-attempt",
        "task-run-attempt",
        LoopBudget::new(2, 0, 1, 1, 0, 1, 1, 1),
    )
    .expect("loop starts");

    let attempt_id = controller
        .current_attempt_id()
        .expect("initial attempt is bound")
        .to_owned();
    controller
        .run_observation("step-attempt", "observed")
        .expect("observation succeeds");
    let steps_used = controller.budget().steps_used;
    controller.complete().expect("loop completes");

    assert_eq!(controller.task_run().status, TaskRunStatus::Succeeded);
    assert_eq!(
        controller.task_run().current_attempt_id.as_deref(),
        Some(attempt_id.as_str())
    );
    assert_eq!(controller.task_run().attempts.len(), 1);
    assert_eq!(
        controller.task_run().attempts[0].status,
        AttemptStatus::Succeeded
    );
    assert_eq!(controller.budget().steps_used, steps_used);
}

fn steering_command(
    command_id: &str,
    project_id: &str,
    goal_id: &str,
    task_run_id: &str,
    loop_run_id: &str,
    kind: CommandKind,
) -> UserCommand {
    UserCommand::new(
        command_id,
        "operator-1",
        project_id,
        goal_id,
        task_run_id,
        loop_run_id,
        kind,
        format!("corr-{command_id}"),
        serde_json::json!({"reason": "operator request"}),
    )
    .expect("steering command is valid")
}

struct ScriptedReader {
    events: VecDeque<Result<CommandRead, UserCommandInboxError>>,
}

impl ScriptedReader {
    fn new(events: impl IntoIterator<Item = Result<CommandRead, UserCommandInboxError>>) -> Self {
        Self {
            events: events.into_iter().collect(),
        }
    }
}

impl NonBlockingCommandReader for ScriptedReader {
    fn read_available(&mut self) -> Result<CommandRead, UserCommandInboxError> {
        self.events.pop_front().unwrap_or_else(|| {
            Err(UserCommandInboxError::Io {
                message: "scripted reader exhausted".into(),
            })
        })
    }
}

fn command_poller(command: &UserCommand) -> CommandPoller<ScriptedReader> {
    let mut input = serde_json::to_vec(command).expect("command serializes");
    input.push(b'\n');
    CommandPoller::new(ScriptedReader::new([Ok(CommandRead::Data(input))]))
}

#[test]
fn controller_polls_and_submits_before_next_safe_boundary_applies_pause_and_resume() {
    let directory = test_data_dir();
    fs::create_dir_all(&directory).expect("test directory creates");
    let inbox = UserCommandInbox::open(directory.join("commands.jsonl"));
    let goal = GoalContract::new(
        "goal-polled-steering",
        "project-polled-steering",
        "inspect repository",
        vec![".".into()],
        Vec::new(),
        vec!["manual_review".into()],
    );
    let mut controller = ReadOnlyLoopController::start(
        EventLog::open(directory.join("events.jsonl")).expect("event log opens"),
        goal,
        "loop-polled-steering",
        "task-run-polled-steering",
        LoopBudget::new(2, 0, 0, 1, 0, 1, 1, 1),
    )
    .expect("loop starts")
    .with_command_inbox(inbox.clone());

    let pause = steering_command(
        "pause-polled",
        "project-polled-steering",
        "goal-polled-steering",
        "task-run-polled-steering",
        "loop-polled-steering",
        CommandKind::Pause,
    );
    let mut pause_poller = command_poller(&pause);
    assert_eq!(
        controller
            .poll_and_submit_command(&mut pause_poller)
            .expect("pause poll submits"),
        CommandPollResult::Command(pause)
    );
    assert_eq!(
        inbox
            .latest("pause-polled")
            .expect("pause is persisted")
            .status,
        CommandStatus::Pending
    );
    assert_eq!(controller.status(), LoopStatus::Running);

    let budget_before_pause = controller.budget().clone();
    assert!(matches!(
        controller.run_observation("step-after-pause", "not executed"),
        Err(LoopControllerError::Paused)
    ));
    assert_eq!(controller.budget(), &budget_before_pause);
    assert_eq!(
        inbox
            .latest("pause-polled")
            .expect("pause status exists")
            .status,
        CommandStatus::Applied
    );

    let resume = steering_command(
        "resume-polled",
        "project-polled-steering",
        "goal-polled-steering",
        "task-run-polled-steering",
        "loop-polled-steering",
        CommandKind::Resume,
    );
    let mut resume_poller = command_poller(&resume);
    assert_eq!(
        controller
            .poll_and_submit_command(&mut resume_poller)
            .expect("resume poll submits"),
        CommandPollResult::Command(resume)
    );
    controller
        .run_observation("step-after-resume", "resumed safely")
        .expect("resume applies before the next step");
    assert_eq!(controller.status(), LoopStatus::Running);
    assert_eq!(controller.budget().steps_used, 1);
    assert_eq!(
        inbox
            .latest("resume-polled")
            .expect("resume status exists")
            .status,
        CommandStatus::Applied
    );
}

#[test]
fn controller_batch_poll_submits_at_most_limit_in_order() {
    let directory = test_data_dir();
    fs::create_dir_all(&directory).expect("test directory creates");
    let inbox = UserCommandInbox::open(directory.join("commands.jsonl"));
    let mut controller = ReadOnlyLoopController::start(
        EventLog::open(directory.join("events.jsonl")).expect("event log opens"),
        GoalContract::new(
            "goal-batch-poll",
            "project-batch-poll",
            "inspect repository",
            vec![".".into()],
            Vec::new(),
            vec!["manual_review".into()],
        ),
        "loop-batch-poll",
        "task-run-batch-poll",
        LoopBudget::new(2, 0, 0, 1, 0, 1, 1, 1),
    )
    .expect("loop starts")
    .with_command_inbox(inbox.clone());

    let commands = [
        steering_command(
            "batch-1",
            "project-batch-poll",
            "goal-batch-poll",
            "task-run-batch-poll",
            "loop-batch-poll",
            CommandKind::Pause,
        ),
        steering_command(
            "batch-2",
            "project-batch-poll",
            "goal-batch-poll",
            "task-run-batch-poll",
            "loop-batch-poll",
            CommandKind::Resume,
        ),
        steering_command(
            "batch-3",
            "project-batch-poll",
            "goal-batch-poll",
            "task-run-batch-poll",
            "loop-batch-poll",
            CommandKind::Cancel,
        ),
    ];
    let mut input = Vec::new();
    for command in &commands {
        input.extend(serde_json::to_vec(command).expect("command serializes"));
        input.push(b'\n');
    }
    let mut poller = CommandPoller::new(ScriptedReader::new([
        Ok(CommandRead::Data(input)),
        Ok(CommandRead::NoInput),
    ]));

    let first = controller
        .poll_and_submit_commands(&mut poller, 2)
        .expect("first batch submits");
    assert_eq!(
        first
            .commands
            .iter()
            .map(|command| command.command_id.as_str())
            .collect::<Vec<_>>(),
        vec!["batch-1", "batch-2"]
    );
    assert_eq!(first.stop_reason, CommandDrainStopReason::LimitReached);
    assert_eq!(
        inbox
            .latest("batch-1")
            .expect("first command exists")
            .status,
        CommandStatus::Pending
    );
    assert_eq!(
        inbox
            .latest("batch-2")
            .expect("second command exists")
            .status,
        CommandStatus::Pending
    );
    assert!(inbox.latest("batch-3").is_err());

    let second = controller
        .poll_and_submit_commands(&mut poller, 2)
        .expect("second batch submits");
    assert_eq!(
        second
            .commands
            .iter()
            .map(|command| command.command_id.as_str())
            .collect::<Vec<_>>(),
        vec!["batch-3"]
    );
    assert_eq!(second.stop_reason, CommandDrainStopReason::NoInput);
    assert_eq!(
        inbox
            .latest("batch-3")
            .expect("third command exists")
            .status,
        CommandStatus::Pending
    );
}

#[test]
fn controller_poll_and_apply_commands_limits_and_applies_in_order() {
    let directory = test_data_dir();
    fs::create_dir_all(&directory).expect("test directory creates");
    let events_path = directory.join("events.jsonl");
    let inbox = UserCommandInbox::open(directory.join("commands.jsonl"));
    let mut controller = ReadOnlyLoopController::start(
        EventLog::open(&events_path).expect("event log opens"),
        GoalContract::new(
            "goal-tick-apply",
            "project-tick-apply",
            "inspect repository",
            vec![".".into()],
            Vec::new(),
            vec!["manual_review".into()],
        ),
        "loop-tick-apply",
        "task-run-tick-apply",
        LoopBudget::new(2, 0, 0, 1, 0, 1, 1, 1),
    )
    .expect("loop starts")
    .with_command_inbox(inbox.clone());

    let commands = [
        steering_command(
            "tick-apply-1",
            "project-tick-apply",
            "goal-tick-apply",
            "task-run-tick-apply",
            "loop-tick-apply",
            CommandKind::Pause,
        ),
        steering_command(
            "tick-apply-2",
            "project-tick-apply",
            "goal-tick-apply",
            "task-run-tick-apply",
            "loop-tick-apply",
            CommandKind::Resume,
        ),
        steering_command(
            "tick-apply-3",
            "project-tick-apply",
            "goal-tick-apply",
            "task-run-tick-apply",
            "loop-tick-apply",
            CommandKind::Cancel,
        ),
    ];
    let mut input = Vec::new();
    for command in &commands {
        input.extend(serde_json::to_vec(command).expect("command serializes"));
        input.push(b'\n');
    }
    let mut poller = CommandPoller::new(ScriptedReader::new([
        Ok(CommandRead::Data(input)),
        Ok(CommandRead::NoInput),
    ]));

    let first = controller
        .poll_and_apply_commands(&mut poller, 2)
        .expect("first tick applies");
    assert_eq!(first.submitted_count, 2);
    assert_eq!(first.applied_count, 2);
    assert_eq!(first.rejected_count, 0);
    assert_eq!(first.stop_reason, CommandDrainStopReason::LimitReached);
    assert_eq!(
        inbox
            .latest("tick-apply-1")
            .expect("first command exists")
            .status,
        CommandStatus::Applied
    );
    assert_eq!(
        inbox
            .latest("tick-apply-2")
            .expect("second command exists")
            .status,
        CommandStatus::Applied
    );
    assert!(inbox.latest("tick-apply-3").is_err());
    assert_eq!(controller.status(), LoopStatus::Running);

    let second = controller
        .poll_and_apply_commands(&mut poller, 2)
        .expect("second tick applies");
    assert_eq!(second.submitted_count, 1);
    assert_eq!(second.applied_count, 1);
    assert_eq!(second.rejected_count, 0);
    assert_eq!(second.stop_reason, CommandDrainStopReason::NoInput);
    assert_eq!(
        inbox
            .latest("tick-apply-3")
            .expect("third command exists")
            .status,
        CommandStatus::Applied
    );
    assert_eq!(controller.status(), LoopStatus::Cancelled);

    let events = EventLog::open(events_path)
        .expect("event log reopens")
        .read_all()
        .expect("events read");
    let applied_ids = events
        .iter()
        .filter(|event| event.event_type == "user_command_applied")
        .filter_map(|event| event.payload["command_id"].as_str())
        .collect::<Vec<_>>();
    assert_eq!(
        applied_ids,
        vec!["tick-apply-1", "tick-apply-2", "tick-apply-3"]
    );
}

#[test]
fn controller_does_not_submit_eof_partial_or_invalid_poll_results() {
    let directory = test_data_dir();
    fs::create_dir_all(&directory).expect("test directory creates");
    let inbox_path = directory.join("commands.jsonl");
    let inbox = UserCommandInbox::open(&inbox_path);
    let mut controller = ReadOnlyLoopController::start(
        EventLog::open(directory.join("events.jsonl")).expect("event log opens"),
        GoalContract::new(
            "goal-poll-errors",
            "project-poll-errors",
            "inspect repository",
            vec![".".into()],
            Vec::new(),
            vec!["manual_review".into()],
        ),
        "loop-poll-errors",
        "task-run-poll-errors",
        LoopBudget::new(1, 0, 0, 1, 0, 1, 1, 1),
    )
    .expect("loop starts")
    .with_command_inbox(inbox);

    let mut eof = CommandPoller::new(ScriptedReader::new([Ok(CommandRead::Eof)]));
    assert_eq!(
        controller
            .poll_and_submit_command(&mut eof)
            .expect("eof is a non-command result"),
        CommandPollResult::Eof
    );

    let mut partial = CommandPoller::new(ScriptedReader::new([
        Ok(CommandRead::Data(b"{\"command_id\":".to_vec())),
        Ok(CommandRead::Eof),
    ]));
    assert!(matches!(
        controller.poll_and_submit_command(&mut partial),
        Err(LoopControllerError::UserCommandInbox(
            UserCommandInboxError::PartialLineAtEof
        ))
    ));

    let mut invalid = CommandPoller::new(ScriptedReader::new([Ok(CommandRead::Data(
        b"not-json\n".to_vec(),
    ))]));
    assert!(matches!(
        controller.poll_and_submit_command(&mut invalid),
        Err(LoopControllerError::UserCommandInbox(
            UserCommandInboxError::InvalidJson { line: 1, .. }
        ))
    ));
    assert!(!inbox_path.exists());
}

#[test]
fn polling_without_an_inbox_fails_closed_without_persisting_a_command() {
    let directory = test_data_dir();
    fs::create_dir_all(&directory).expect("test directory creates");
    let command_path = directory.join("commands.jsonl");
    let command = steering_command(
        "no-inbox",
        "project-no-inbox",
        "goal-no-inbox",
        "task-run-no-inbox",
        "loop-no-inbox",
        CommandKind::Pause,
    );
    let mut poller = command_poller(&command);
    let mut controller = ReadOnlyLoopController::start(
        EventLog::open(directory.join("events.jsonl")).expect("event log opens"),
        GoalContract::new(
            "goal-no-inbox",
            "project-no-inbox",
            "inspect repository",
            vec![".".into()],
            Vec::new(),
            vec!["manual_review".into()],
        ),
        "loop-no-inbox",
        "task-run-no-inbox",
        LoopBudget::new(1, 0, 0, 1, 0, 1, 1, 1),
    )
    .expect("loop starts");

    assert!(matches!(
        controller.poll_and_submit_command(&mut poller),
        Err(LoopControllerError::CommandInboxRequired)
    ));
    assert!(!command_path.exists());
}

#[test]
fn polled_takeover_is_rejected_without_changing_attempt_or_budget() {
    let directory = test_data_dir();
    fs::create_dir_all(&directory).expect("test directory creates");
    let inbox = UserCommandInbox::open(directory.join("commands.jsonl"));
    let mut controller = ReadOnlyLoopController::start(
        EventLog::open(directory.join("events.jsonl")).expect("event log opens"),
        GoalContract::new(
            "goal-takeover",
            "project-takeover",
            "inspect repository",
            vec![".".into()],
            Vec::new(),
            vec!["manual_review".into()],
        ),
        "loop-takeover",
        "task-run-takeover",
        LoopBudget::new(1, 0, 0, 1, 0, 1, 1, 1),
    )
    .expect("loop starts")
    .with_command_inbox(inbox.clone());
    let attempt_before = controller
        .current_attempt_id()
        .expect("attempt exists")
        .to_owned();
    let budget_before = controller.budget().clone();
    let takeover = steering_command(
        "takeover-polled",
        "project-takeover",
        "goal-takeover",
        "task-run-takeover",
        "loop-takeover",
        CommandKind::Takeover,
    );
    let mut poller = command_poller(&takeover);
    assert!(matches!(
        controller.poll_and_submit_command(&mut poller),
        Ok(CommandPollResult::Command(_))
    ));

    controller
        .run_observation("step-after-takeover", "normal action remains allowed")
        .expect("rejected takeover does not stop normal loop action");
    assert_eq!(controller.status(), LoopStatus::Running);
    assert_eq!(
        controller.current_attempt_id(),
        Some(attempt_before.as_str())
    );
    assert_eq!(controller.budget().steps_used, budget_before.steps_used + 1);
    assert_eq!(
        inbox
            .latest("takeover-polled")
            .expect("takeover status exists")
            .status,
        CommandStatus::Rejected
    );
}

#[test]
fn polled_cancel_is_applied_at_the_next_boundary_without_consuming_budget() {
    let directory = test_data_dir();
    fs::create_dir_all(&directory).expect("test directory creates");
    let inbox = UserCommandInbox::open(directory.join("commands.jsonl"));
    let mut controller = ReadOnlyLoopController::start(
        EventLog::open(directory.join("events.jsonl")).expect("event log opens"),
        GoalContract::new(
            "goal-cancel-polled",
            "project-cancel-polled",
            "inspect repository",
            vec![".".into()],
            Vec::new(),
            vec!["manual_review".into()],
        ),
        "loop-cancel-polled",
        "task-run-cancel-polled",
        LoopBudget::new(1, 0, 0, 1, 0, 1, 1, 1),
    )
    .expect("loop starts")
    .with_command_inbox(inbox.clone());
    let budget_before = controller.budget().clone();
    let cancel = steering_command(
        "cancel-polled",
        "project-cancel-polled",
        "goal-cancel-polled",
        "task-run-cancel-polled",
        "loop-cancel-polled",
        CommandKind::Cancel,
    );
    let mut poller = command_poller(&cancel);
    assert!(matches!(
        controller.poll_and_submit_command(&mut poller),
        Ok(CommandPollResult::Command(_))
    ));

    assert!(matches!(
        controller.run_observation("step-after-cancel", "not executed"),
        Err(LoopControllerError::Terminal)
    ));
    assert_eq!(controller.status(), LoopStatus::Cancelled);
    assert_eq!(controller.budget(), &budget_before);
    assert_eq!(
        inbox
            .latest("cancel-polled")
            .expect("cancel status exists")
            .status,
        CommandStatus::Applied
    );
}

#[test]
fn pause_command_is_applied_at_next_safe_boundary_and_resume_reopens_loop() {
    let directory = test_data_dir();
    fs::create_dir_all(&directory).expect("test directory creates");
    let events_path = directory.join("events.jsonl");
    let inbox_path = directory.join("commands.jsonl");
    let inbox = UserCommandInbox::open(&inbox_path);
    inbox
        .submit(steering_command(
            "pause-1",
            "project-steering",
            "goal-steering",
            "task-run-steering",
            "loop-steering",
            CommandKind::Pause,
        ))
        .expect("pause command submits");
    let goal = GoalContract::new(
        "goal-steering",
        "project-steering",
        "inspect repository",
        vec![".".into()],
        Vec::new(),
        vec!["manual_review".into()],
    );
    let budget = LoopBudget::new(2, 0, 0, 1, 0, 1, 1, 1);
    let mut controller = ReadOnlyLoopController::start(
        EventLog::open(&events_path).expect("event log opens"),
        goal,
        "loop-steering",
        "task-run-steering",
        budget,
    )
    .expect("loop starts")
    .with_command_inbox(inbox.clone());

    assert!(matches!(
        controller.run_observation("step-paused", "not executed"),
        Err(LoopControllerError::Paused)
    ));
    assert_eq!(controller.status(), LoopStatus::Paused);
    assert_eq!(
        inbox.latest("pause-1").expect("pause status exists").status,
        CommandStatus::Applied
    );

    inbox
        .submit(steering_command(
            "resume-1",
            "project-steering",
            "goal-steering",
            "task-run-steering",
            "loop-steering",
            CommandKind::Resume,
        ))
        .expect("resume command submits");
    controller
        .run_observation("step-resumed", "resumed safely")
        .expect("resume command applies before step");
    assert_eq!(controller.status(), LoopStatus::Running);

    let events = EventLog::open(events_path)
        .expect("event log reopens")
        .read_all()
        .expect("events read");
    assert!(events.iter().any(|event| event.event_type == "loop_paused"));
    assert!(events
        .iter()
        .any(|event| event.event_type == "loop_resumed"));
    assert!(events
        .iter()
        .any(|event| event.event_type == "user_command_applied"));
}

#[test]
fn cancel_command_stops_loop_at_safe_boundary_without_logging_payload() {
    let directory = test_data_dir();
    fs::create_dir_all(&directory).expect("test directory creates");
    let events_path = directory.join("events.jsonl");
    let inbox = UserCommandInbox::open(directory.join("commands.jsonl"));
    let mut command = steering_command(
        "cancel-1",
        "project-cancel",
        "goal-cancel",
        "task-run-cancel",
        "loop-cancel",
        CommandKind::Cancel,
    );
    command.payload = serde_json::json!({"reason": "stop", "note": "safe"});
    inbox.submit(command).expect("cancel command submits");
    let goal = GoalContract::new(
        "goal-cancel",
        "project-cancel",
        "inspect repository",
        vec![".".into()],
        Vec::new(),
        vec!["manual_review".into()],
    );
    let budget = LoopBudget::new(1, 0, 0, 1, 0, 1, 1, 1);
    let mut controller = ReadOnlyLoopController::start(
        EventLog::open(&events_path).expect("event log opens"),
        goal,
        "loop-cancel",
        "task-run-cancel",
        budget,
    )
    .expect("loop starts")
    .with_command_inbox(inbox.clone());

    assert!(matches!(
        controller.run_observation("step-cancelled", "not executed"),
        Err(LoopControllerError::Terminal)
    ));
    assert_eq!(controller.status(), LoopStatus::Cancelled);
    assert_eq!(
        inbox
            .latest("cancel-1")
            .expect("cancel status exists")
            .status,
        CommandStatus::Applied
    );
    let encoded = fs::read_to_string(events_path).expect("events readable");
    assert!(!encoded.contains("safe"));
}

#[test]
fn readonly_loop_records_start_step_and_completion_events() {
    let directory = test_data_dir();
    fs::create_dir_all(&directory).expect("test directory creates");
    let log = EventLog::open(directory.join("events.jsonl")).expect("event log opens");
    let goal = GoalContract::new(
        "goal-1",
        "project-1",
        "inspect repository",
        vec![".".into()],
        Vec::new(),
        vec!["manual_review".into()],
    );
    let budget = LoopBudget::new(1, 0, 0, 1, 0, 1, 1, 1);
    let mut controller = ReadOnlyLoopController::start(log, goal, "loop-1", "task-run-1", budget)
        .expect("loop starts");
    let step = controller
        .run_observation("step-1", "repository inspected")
        .expect("observation completes");
    let run = controller.complete().expect("loop completes");
    assert_eq!(step.step_no, 1);
    assert!(step.is_terminal());
    assert_eq!(run.status, LoopStatus::Completed);
    let events = EventLog::open(directory.join("events.jsonl"))
        .expect("event log reopens")
        .read_all()
        .expect("events read");
    assert_eq!(events.len(), 4);
    assert_eq!(events[0].event_type, "loop_started");
    assert_eq!(events[1].event_type, "step_started");
    assert_eq!(events[2].event_type, "step_completed");
    assert_eq!(events[3].event_type, "loop_completed");
    assert_eq!(events[1].loop_run_id.as_deref(), Some("loop-1"));
    assert_eq!(events[1].step_id.as_deref(), Some("step-1"));
    assert_eq!(events[2].sequence_no, Some(3));
}

#[test]
fn readonly_loop_stops_when_step_budget_is_exhausted() {
    let directory = test_data_dir();
    fs::create_dir_all(&directory).expect("test directory creates");
    let log = EventLog::open(directory.join("events.jsonl")).expect("event log opens");
    let goal = GoalContract::new(
        "goal-2",
        "project-2",
        "inspect repository",
        vec![".".into()],
        Vec::new(),
        vec!["manual_review".into()],
    );
    let budget = LoopBudget::new(0, 0, 0, 1, 0, 1, 1, 1);
    let mut controller = ReadOnlyLoopController::start(log, goal, "loop-2", "task-run-2", budget)
        .expect("loop starts");
    assert!(matches!(
        controller.run_observation("step-1", "not run"),
        Err(LoopControllerError::BudgetExhausted(_))
    ));
    let events = EventLog::open(directory.join("events.jsonl"))
        .expect("event log reopens")
        .read_all()
        .expect("events read");
    assert_eq!(events.len(), 2);
    assert_eq!(events[1].event_type, "loop_budget_exhausted");
}

#[test]
fn fixture_model_complete_decision_records_model_events_and_completes_loop() {
    let directory = test_data_dir();
    fs::create_dir_all(&directory).expect("test directory creates");
    let log = EventLog::open(directory.join("events.jsonl")).expect("event log opens");
    let goal = GoalContract::new(
        "goal-model",
        "project-model",
        "inspect repository",
        vec![".".into()],
        Vec::new(),
        vec!["manual_review".into()],
    );
    let budget = LoopBudget::new(1, 0, 1, 1, 0, 1, 1, 1);
    let mut controller =
        ReadOnlyLoopController::start(log, goal, "loop-model", "task-run-model", budget)
            .expect("loop starts");
    let adapter = LocalFixtureModelAdapter::new(ModelDecision::CompleteLoop, "fixture complete");
    let decision = controller
        .run_model_step(
            &adapter,
            "local-fixture",
            "step-model",
            "repository inspected",
        )
        .expect("model step completes");
    assert_eq!(decision, ModelDecision::CompleteLoop);
    let events = EventLog::open(directory.join("events.jsonl"))
        .expect("event log reopens")
        .read_all()
        .expect("events read");
    assert_eq!(events[1].event_type, "step_started");
    assert_eq!(events[2].event_type, "model_call_started");
    assert_eq!(events[3].event_type, "model_call_completed");
    assert_eq!(events[4].event_type, "step_completed");
    assert_eq!(events[5].event_type, "loop_completed");
}

#[test]
fn fixture_model_human_decision_records_intervention_event_without_side_effects() {
    let directory = test_data_dir();
    fs::create_dir_all(&directory).expect("test directory creates");
    let log = EventLog::open(directory.join("events.jsonl")).expect("event log opens");
    let goal = GoalContract::new(
        "goal-human",
        "project-human",
        "inspect repository",
        vec![".".into()],
        Vec::new(),
        vec!["manual_review".into()],
    );
    let budget = LoopBudget::new(1, 0, 1, 1, 0, 1, 1, 1);
    let mut controller =
        ReadOnlyLoopController::start(log, goal, "loop-human", "task-run-human", budget)
            .expect("loop starts");
    let adapter = LocalFixtureModelAdapter::new(ModelDecision::RequestHuman, "approval required");
    let decision = controller
        .run_model_step(
            &adapter,
            "local-fixture",
            "step-human",
            "reviewed high-risk request",
        )
        .expect("model step completes");
    assert_eq!(decision, ModelDecision::RequestHuman);
    let events = EventLog::open(directory.join("events.jsonl"))
        .expect("event log reopens")
        .read_all()
        .expect("events read");
    assert_eq!(events[1].event_type, "step_started");
    assert_eq!(events[3].event_type, "model_call_completed");
    assert_eq!(events[4].event_type, "human_intervention_required");
    assert_eq!(events[5].event_type, "step_completed");
}

#[test]
fn fixture_model_rejects_empty_model_id_and_records_failure_event() {
    let directory = test_data_dir();
    fs::create_dir_all(&directory).expect("test directory creates");
    let log = EventLog::open(directory.join("events.jsonl")).expect("event log opens");
    let goal = GoalContract::new(
        "goal-error",
        "project-error",
        "inspect repository",
        vec![".".into()],
        Vec::new(),
        vec!["manual_review".into()],
    );
    let budget = LoopBudget::new(1, 0, 1, 1, 0, 1, 1, 1);
    let mut controller =
        ReadOnlyLoopController::start(log, goal, "loop-error", "task-run-error", budget)
            .expect("loop starts");
    let adapter = LocalFixtureModelAdapter::new(ModelDecision::ContinueObservation, "not returned");
    assert!(matches!(
        controller.run_model_step(&adapter, "", "step-error", "observation"),
        Err(LoopControllerError::ModelAdapter(_))
    ));
    let events = EventLog::open(directory.join("events.jsonl"))
        .expect("event log reopens")
        .read_all()
        .expect("events read");
    assert_eq!(events[3].event_type, "model_call_failed");
}

#[test]
fn fixture_model_continue_decision_leaves_loop_running_and_records_step_events() {
    let directory = test_data_dir();
    fs::create_dir_all(&directory).expect("test directory creates");
    let log = EventLog::open(directory.join("events.jsonl")).expect("event log opens");
    let goal = GoalContract::new(
        "goal-continue",
        "project-continue",
        "inspect repository",
        vec![".".into()],
        Vec::new(),
        vec!["manual_review".into()],
    );
    let budget = LoopBudget::new(1, 0, 1, 1, 0, 1, 1, 1);
    let mut controller =
        ReadOnlyLoopController::start(log, goal, "loop-continue", "task-run-continue", budget)
            .expect("loop starts");
    let adapter = LocalFixtureModelAdapter::new(
        ModelDecision::ContinueObservation,
        "more observation needed",
    );
    let decision = controller
        .run_model_step(
            &adapter,
            "local-fixture",
            "step-continue",
            "initial observation",
        )
        .expect("model step completes");
    assert_eq!(decision, ModelDecision::ContinueObservation);
    assert_eq!(controller.status(), LoopStatus::Running);
    let events = EventLog::open(directory.join("events.jsonl"))
        .expect("event log reopens")
        .read_all()
        .expect("events read");
    assert_eq!(events[1].event_type, "step_started");
    assert_eq!(events[2].event_type, "model_call_started");
    assert_eq!(events[3].event_type, "model_call_completed");
    assert_eq!(events[4].event_type, "step_completed");
}

#[test]
fn fixture_model_respects_model_call_budget_across_steps() {
    let directory = test_data_dir();
    fs::create_dir_all(&directory).expect("test directory creates");
    let log = EventLog::open(directory.join("events.jsonl")).expect("event log opens");
    let goal = GoalContract::new(
        "goal-model-budget",
        "project-model-budget",
        "inspect repository",
        vec![".".into()],
        Vec::new(),
        vec!["manual_review".into()],
    );
    let budget = LoopBudget::new(2, 0, 1, 1, 0, 1, 1, 1);
    let mut controller = ReadOnlyLoopController::start(
        log,
        goal,
        "loop-model-budget",
        "task-run-model-budget",
        budget,
    )
    .expect("loop starts");
    let adapter =
        LocalFixtureModelAdapter::new(ModelDecision::ContinueObservation, "one model call allowed");
    controller
        .run_model_step(&adapter, "local-fixture", "step-1", "first observation")
        .expect("first model step completes");
    assert!(matches!(
        controller.run_model_step(&adapter, "local-fixture", "step-2", "second observation"),
        Err(LoopControllerError::BudgetExhausted(BudgetKind::ModelCalls))
    ));
    let events = EventLog::open(directory.join("events.jsonl"))
        .expect("event log reopens")
        .read_all()
        .expect("events read");
    assert_eq!(
        events.last().expect("budget event exists").event_type,
        "loop_budget_exhausted"
    );
    assert_eq!(
        events.last().expect("budget event exists").payload["budget_kind"],
        "ModelCalls"
    );
}

#[test]
fn fixture_model_stops_repeated_no_progress_actions() {
    let directory = test_data_dir();
    fs::create_dir_all(&directory).expect("test directory creates");
    let log = EventLog::open(directory.join("events.jsonl")).expect("event log opens");
    let goal = GoalContract::new(
        "goal-no-progress",
        "project-no-progress",
        "inspect repository",
        vec![".".into()],
        Vec::new(),
        vec!["manual_review".into()],
    );
    let budget = LoopBudget::new(3, 0, 3, 1, 0, 3, 2, 1);
    let mut controller = ReadOnlyLoopController::start(
        log,
        goal,
        "loop-no-progress",
        "task-run-no-progress",
        budget,
    )
    .expect("loop starts");
    let adapter = LocalFixtureModelAdapter::new(
        ModelDecision::ContinueObservation,
        "same action with no new evidence",
    );
    controller
        .run_model_step(&adapter, "local-fixture", "step-1", "same observation")
        .expect("first step completes");
    assert!(matches!(
        controller.run_model_step(&adapter, "local-fixture", "step-2", "same observation"),
        Err(LoopControllerError::BudgetExhausted(
            BudgetKind::SameActionRepetitions
        ))
    ));
    let events = EventLog::open(directory.join("events.jsonl"))
        .expect("event log reopens")
        .read_all()
        .expect("events read");
    assert_eq!(
        events.last().expect("no-progress event exists").event_type,
        "no_progress_detected"
    );
}

#[test]
fn controller_records_policy_and_tool_request_for_allowed_read_only_tool() {
    let directory = test_data_dir();
    fs::create_dir_all(&directory).expect("test directory creates");
    let log = EventLog::open(directory.join("events.jsonl")).expect("event log opens");
    let mut goal = GoalContract::new(
        "goal-tool-request",
        "project-tool-request",
        "inspect repository",
        vec![".".into()],
        Vec::new(),
        vec!["git status".into()],
    );
    goal.allowed_tools = vec!["git_status".into()];
    let budget = LoopBudget::new(1, 1, 1, 1, 0, 1, 1, 1);
    let mut controller = ReadOnlyLoopController::start(
        log,
        goal,
        "loop-tool-request",
        "task-run-tool-request",
        budget,
    )
    .expect("loop starts");
    controller
        .request_tool_call(
            &ToolRegistry::baseline().expect("baseline tools register"),
            &PolicyGate::new(),
            "git_status",
            "step-tool-request",
        )
        .expect("read-only tool request is allowed");
    let events = EventLog::open(directory.join("events.jsonl"))
        .expect("event log reopens")
        .read_all()
        .expect("events read");
    assert_eq!(events[1].event_type, "policy_decision");
    assert_eq!(events[1].payload["decision"], "allow");
    assert_eq!(events[2].event_type, "tool_call_requested");
    assert_eq!(events[2].payload["tool_id"], "git_status");
}

#[test]
fn controller_records_policy_and_denies_forbidden_tool_request() {
    let directory = test_data_dir();
    fs::create_dir_all(&directory).expect("test directory creates");
    let log = EventLog::open(directory.join("events.jsonl")).expect("event log opens");
    let mut goal = GoalContract::new(
        "goal-tool-denied",
        "project-tool-denied",
        "inspect repository",
        vec![".".into()],
        Vec::new(),
        vec!["manual_review".into()],
    );
    goal.allowed_tools = vec!["git_status".into()];
    goal.forbidden_tools = vec!["git_status".into()];
    let budget = LoopBudget::new(1, 1, 1, 1, 0, 1, 1, 1);
    let mut controller = ReadOnlyLoopController::start(
        log,
        goal,
        "loop-tool-denied",
        "task-run-tool-denied",
        budget,
    )
    .expect("loop starts");
    assert!(matches!(
        controller.request_tool_call(
            &ToolRegistry::baseline().expect("baseline tools register"),
            &PolicyGate::new(),
            "git_status",
            "step-tool-denied",
        ),
        Err(LoopControllerError::PolicyDenied {
            decision: PolicyDecision::Block,
            ..
        })
    ));
    let events = EventLog::open(directory.join("events.jsonl"))
        .expect("event log reopens")
        .read_all()
        .expect("events read");
    assert_eq!(events.len(), 2);
    assert_eq!(events[1].event_type, "policy_decision");
    assert_eq!(events[1].payload["decision"], "block");
}

#[test]
fn controller_executes_allowed_git_status_and_records_completion() {
    let directory = test_data_dir();
    fs::create_dir_all(&directory).expect("test directory creates");
    let log = EventLog::open(directory.join("events.jsonl")).expect("event log opens");
    let mut goal = GoalContract::new(
        "goal-tool-execution",
        "project-tool-execution",
        "inspect repository",
        vec![".".into()],
        Vec::new(),
        vec!["git status".into()],
    );
    goal.allowed_tools = vec!["git_status".into()];
    let budget = LoopBudget::new(1, 1, 1, 1, 0, 1, 1, 1);
    let mut controller = ReadOnlyLoopController::start(
        log,
        goal,
        "loop-tool-execution",
        "task-run-tool-execution",
        budget,
    )
    .expect("loop starts");
    let workspace = std::env::current_dir().expect("current directory exists");
    let output = controller
        .execute_read_only_tool(
            &ToolRegistry::baseline().expect("baseline tools register"),
            &PolicyGate::new(),
            &ReadOnlyToolExecutor,
            &workspace,
            ToolCallInput {
                tool_id: "git_status".into(),
                arguments: serde_json::json!({}),
            },
            "step-tool-execution",
        )
        .expect("allowed read-only tool executes");
    assert!(output.result["executed"].as_bool().unwrap());
    let events = EventLog::open(directory.join("events.jsonl"))
        .expect("event log reopens")
        .read_all()
        .expect("events read");
    assert_eq!(events[1].event_type, "policy_decision");
    assert_eq!(events[2].event_type, "tool_call_requested");
    assert_eq!(events[3].event_type, "tool_call_completed");
    assert_eq!(events[3].payload["tool_id"], "git_status");
}

#[test]
fn controller_persists_tool_trace_and_explicit_no_side_effect_record() {
    let directory = test_data_dir();
    fs::create_dir_all(&directory).expect("test directory creates");
    let log = EventLog::open(directory.join("events.jsonl")).expect("event log opens");
    let mut goal = GoalContract::new(
        "goal-tool-audit",
        "project-tool-audit",
        "inspect repository",
        vec![".".into()],
        Vec::new(),
        vec!["git status".into()],
    );
    goal.allowed_tools = vec!["git_status".into()];
    let mut controller = ReadOnlyLoopController::start(
        log,
        goal,
        "loop-tool-audit",
        "task-run-tool-audit",
        LoopBudget::new(1, 1, 1, 1, 0, 1, 1, 1),
    )
    .expect("loop starts")
    .with_audit_stores(
        directory.join("traces.jsonl"),
        directory.join("side-effects.jsonl"),
    );
    let workspace = std::env::current_dir().expect("current directory exists");
    controller
        .execute_read_only_tool(
            &ToolRegistry::baseline().expect("baseline tools register"),
            &PolicyGate::new(),
            &ReadOnlyToolExecutor,
            &workspace,
            ToolCallInput {
                tool_id: "git_status".into(),
                arguments: serde_json::json!({}),
            },
            "step-tool-audit",
        )
        .expect("read-only tool executes");
    let traces = TraceStore::open(directory.join("traces.jsonl"))
        .read_all()
        .expect("trace reads");
    let side_effects = SideEffectLedger::open(directory.join("side-effects.jsonl"))
        .read_all()
        .expect("side effect ledger reads");
    assert_eq!(traces.len(), 1);
    assert_eq!(traces[0].step_id.as_deref(), Some("step-tool-audit"));
    assert_eq!(side_effects.len(), 1);
    assert_eq!(side_effects[0].class, "none");
    assert_eq!(side_effects[0].status, "completed");
}

#[test]
fn controller_redacts_raw_search_output_from_persisted_audit_records() {
    let directory = test_data_dir();
    let workspace = directory.join("workspace");
    fs::create_dir_all(&workspace).expect("workspace creates");
    let marker = "FAKE_SECRET_MARKER_FOR_AUDIT_TEST";
    fs::write(workspace.join("evidence.txt"), format!("{marker}\n"))
        .expect("fixture evidence writes");

    let mut goal = GoalContract::new(
        "goal-redaction",
        "project-redaction",
        "inspect repository",
        vec![".".into()],
        Vec::new(),
        vec!["search completed".into()],
    );
    goal.allowed_tools = vec!["rg_fixed".into()];
    let mut controller = ReadOnlyLoopController::start(
        EventLog::open(directory.join("events.jsonl")).expect("event log opens"),
        goal,
        "loop-redaction",
        "task-run-redaction",
        LoopBudget::new(1, 1, 1, 1, 0, 1, 1, 1),
    )
    .expect("loop starts")
    .with_audit_stores(
        directory.join("traces.jsonl"),
        directory.join("side-effects.jsonl"),
    );

    controller
        .execute_read_only_tool(
            &ToolRegistry::baseline().expect("baseline tools register"),
            &PolicyGate::new(),
            &ReadOnlyToolExecutor,
            &workspace,
            ToolCallInput {
                tool_id: "rg_fixed".into(),
                arguments: serde_json::json!({
                    "path": "evidence.txt",
                    "query": marker,
                }),
            },
            "step-redaction",
        )
        .expect("search tool executes");

    let events = EventLog::open(directory.join("events.jsonl"))
        .expect("event log reopens")
        .read_all()
        .expect("events read");
    let traces = TraceStore::open(directory.join("traces.jsonl"))
        .read_all()
        .expect("traces read");
    let side_effects = SideEffectLedger::open(directory.join("side-effects.jsonl"))
        .read_all()
        .expect("side effects read");
    let persisted = format!("{events:?}{traces:?}{side_effects:?}");
    assert!(!persisted.contains(marker));
    let completed = events
        .iter()
        .find(|event| event.event_type == "tool_call_completed")
        .expect("completed event exists");
    assert_eq!(completed.payload["tool_id"], "rg_fixed");
    assert_eq!(completed.payload["recipe_ref"], "rg-fixed-search");
    assert_eq!(completed.payload["output_redacted"], true);
    assert!(completed.payload.get("stdout").is_none());
    assert!(completed.payload.get("query").is_none());
    assert_eq!(traces[0].summary, "tool call completed; output redacted");
    assert_eq!(
        side_effects[0].summary,
        "tool call completed; output redacted"
    );
}

#[test]
fn controller_records_tool_failure_after_policy_allows_request() {
    let directory = test_data_dir();
    fs::create_dir_all(&directory).expect("test directory creates");
    let log = EventLog::open(directory.join("events.jsonl")).expect("event log opens");
    let mut goal = GoalContract::new(
        "goal-tool-failure",
        "project-tool-failure",
        "inspect repository",
        vec![".".into()],
        Vec::new(),
        vec!["git status".into()],
    );
    goal.allowed_tools = vec!["git_status".into()];
    let budget = LoopBudget::new(1, 1, 1, 1, 0, 1, 1, 1);
    let mut controller = ReadOnlyLoopController::start(
        log,
        goal,
        "loop-tool-failure",
        "task-run-tool-failure",
        budget,
    )
    .expect("loop starts");
    assert!(controller
        .execute_read_only_tool(
            &ToolRegistry::baseline().expect("baseline tools register"),
            &PolicyGate::new(),
            &ReadOnlyToolExecutor,
            std::path::Path::new("/tmp/steerbox-no-such-workspace"),
            ToolCallInput {
                tool_id: "git_status".into(),
                arguments: serde_json::json!({}),
            },
            "step-tool-failure",
        )
        .is_err());
    let events = EventLog::open(directory.join("events.jsonl"))
        .expect("event log reopens")
        .read_all()
        .expect("events read");
    assert_eq!(events[1].event_type, "policy_decision");
    assert_eq!(events[2].event_type, "tool_call_requested");
    assert_eq!(events[3].event_type, "tool_call_failed");
    assert_eq!(events[3].payload["failure_category"], "permission");
    assert_eq!(events[3].payload["recipe_ref"], "git-status-readonly");
    let tool_call_id = events[3].payload["tool_call_id"]
        .as_str()
        .expect("tool call identity is a string");
    assert!(tool_call_id.starts_with("loop-tool-failure:tool:"));
    assert_eq!(
        events[3].payload["lesson_candidate"]["lifecycle_state"],
        "candidate"
    );
    let failure = &events[3];
    let source_event_ids = failure.payload["lesson_candidate"]["source_event_ids"]
        .as_array()
        .expect("lesson source event references exist");
    assert_eq!(source_event_ids.len(), 1);
    for reference in source_event_ids {
        let source_event_id = reference.as_str().expect("source event id is a string");
        assert!(
            events.iter().any(|event| event.event_id == source_event_id),
            "lesson references an event that was never persisted: {source_event_id}"
        );
        assert_eq!(source_event_id, failure.event_id);
    }
    assert!(!events[3].payload.to_string().contains("api_key"));
}
