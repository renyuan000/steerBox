use steerbox::domain::{
    AttemptLineageError, AttemptReason, AttemptStatus, LoopBudget, TaskRun, TaskRunStatus,
};

fn budget(max_attempts: u32) -> LoopBudget {
    LoopBudget::new(max_attempts, 20, 20, max_attempts, 4, 4, 4, 4)
}

#[test]
fn retry_and_fallback_preserve_task_run_budget_and_parent_lineage() {
    let mut task_run = TaskRun::new("run-1", "task-1", budget(3)).expect("task run");
    let initial = task_run
        .start_attempt("attempt-1", AttemptReason::Initial, None)
        .expect("initial attempt");
    task_run.budget.steps_used = 4;
    task_run.budget.model_calls_used = 2;
    task_run
        .finish_attempt(
            &initial.attempt_id,
            AttemptStatus::Failed,
            "failed",
            vec!["artifact://failed".into()],
            vec!["event://failure".into()],
        )
        .expect("finish initial");

    let retry = task_run
        .start_attempt(
            "attempt-2",
            AttemptReason::Retry,
            Some(initial.attempt_id.clone()),
        )
        .expect("retry attempt");
    assert_eq!(retry.attempt_no, 2);
    assert_eq!(retry.parent_attempt_id.as_deref(), Some("attempt-1"));
    assert_eq!(task_run.budget.attempts_used, 2);
    assert_eq!(task_run.budget.steps_used, 4);
    assert_eq!(task_run.budget.model_calls_used, 2);

    task_run
        .finish_attempt(
            &retry.attempt_id,
            AttemptStatus::Failed,
            "fallback_required",
            vec!["artifact://retry".into()],
            vec!["event://retry-failure".into()],
        )
        .expect("finish retry");
    let fallback = task_run
        .start_attempt(
            "attempt-3",
            AttemptReason::Fallback,
            Some(retry.attempt_id.clone()),
        )
        .expect("fallback attempt");
    assert_eq!(fallback.attempt_no, 3);
    assert_eq!(fallback.parent_attempt_id.as_deref(), Some("attempt-2"));
    assert_eq!(task_run.budget.attempts_used, 3);
    assert_eq!(task_run.current_attempt_id.as_deref(), Some("attempt-3"));
    assert_eq!(
        task_run.attempt("attempt-1").unwrap().evidence_refs,
        ["event://failure"]
    );
}

#[test]
fn resume_and_fork_require_finished_parent_and_do_not_reopen_it() {
    let mut task_run = TaskRun::new("run-1", "task-1", budget(3)).expect("task run");
    let initial = task_run
        .start_attempt("attempt-1", AttemptReason::Initial, None)
        .expect("initial attempt");
    let before = task_run.budget.clone();
    let error = task_run
        .start_attempt(
            "attempt-2",
            AttemptReason::Resume,
            Some(initial.attempt_id.clone()),
        )
        .expect_err("running parent must block resume");
    assert_eq!(
        error,
        AttemptLineageError::ParentNotFinished("attempt-1".into())
    );
    assert_eq!(task_run.budget, before);

    task_run
        .finish_attempt(
            &initial.attempt_id,
            AttemptStatus::Cancelled,
            "cancelled",
            Vec::new(),
            vec!["event://cancelled".into()],
        )
        .expect("finish initial");
    let fork = task_run
        .start_attempt(
            "attempt-2",
            AttemptReason::Fork,
            Some(initial.attempt_id.clone()),
        )
        .expect("fork attempt");
    assert_eq!(fork.parent_attempt_id.as_deref(), Some("attempt-1"));
    assert_eq!(
        task_run.attempt("attempt-1").unwrap().status,
        AttemptStatus::Cancelled
    );
}

#[test]
fn invalid_lineage_and_budget_fail_without_mutating_task_run() {
    let mut task_run = TaskRun::new("run-1", "task-1", budget(1)).expect("task run");
    let initial = task_run
        .start_attempt("attempt-1", AttemptReason::Initial, None)
        .expect("initial attempt");
    let snapshot = task_run.clone();

    assert_eq!(
        task_run
            .start_attempt("attempt-2", AttemptReason::Fallback, None)
            .expect_err("fallback needs parent"),
        AttemptLineageError::ParentRequired
    );
    assert_eq!(task_run, snapshot);

    task_run
        .finish_attempt(
            &initial.attempt_id,
            AttemptStatus::Failed,
            "failed",
            Vec::new(),
            Vec::new(),
        )
        .expect("finish initial");
    let snapshot = task_run.clone();
    assert_eq!(
        task_run
            .start_attempt("attempt-2", AttemptReason::Retry, Some(initial.attempt_id),)
            .expect_err("attempt budget is exhausted"),
        AttemptLineageError::AttemptBudgetExhausted
    );
    assert_eq!(task_run, snapshot);
}

#[test]
fn attempt_cannot_be_finished_twice_or_with_running_status() {
    let mut task_run = TaskRun::new("run-1", "task-1", budget(2)).expect("task run");
    let attempt = task_run
        .start_attempt("attempt-1", AttemptReason::Initial, None)
        .expect("initial attempt");
    assert_eq!(
        task_run
            .finish_attempt(
                &attempt.attempt_id,
                AttemptStatus::Running,
                "not_run",
                Vec::new(),
                Vec::new(),
            )
            .expect_err("running is not a terminal result"),
        AttemptLineageError::NonTerminalFinish
    );
    task_run
        .finish_attempt(
            &attempt.attempt_id,
            AttemptStatus::Succeeded,
            "passed",
            vec!["artifact://result".into()],
            vec!["event://verified".into()],
        )
        .expect("finish attempt");
    assert_eq!(
        task_run
            .finish_attempt(
                &attempt.attempt_id,
                AttemptStatus::Failed,
                "failed-again",
                Vec::new(),
                Vec::new(),
            )
            .expect_err("terminal attempt cannot be reopened"),
        AttemptLineageError::AttemptAlreadyFinished("attempt-1".into())
    );
}

#[test]
fn task_run_starts_running_and_rejects_empty_identity() {
    assert_eq!(
        TaskRun::new(" ", "task-1", budget(1)).expect_err("blank run id"),
        AttemptLineageError::EmptyField {
            field: "task_run_id"
        }
    );
    assert_eq!(
        TaskRun::new("run-1", "", budget(1)).expect_err("blank task id"),
        AttemptLineageError::EmptyField { field: "task_id" }
    );
    let task_run = TaskRun::new("run-1", "task-1", budget(1)).expect("task run");
    assert_eq!(task_run.status, TaskRunStatus::Running);
    assert_eq!(task_run.schema_version, 1);
}
