// 中断能力验收测试
//
// 测试目标：
// 1. pause 命令在安全边界响应 < 1s
// 2. 取消命令在下一安全边界生效，不声称中断正在执行的工具或模型调用
// 3. resume 恢复执行
// 4. 当前未实现的 takeover 明确拒绝，不声称接管完成

use std::time::{Duration, Instant};
use steerbox::domain::{GoalContract, LoopBudget, LoopStatus};
use steerbox::event_log::EventLog;
use steerbox::loop_controller::{LoopControllerError, ReadOnlyLoopController};
use steerbox::user_command_inbox::{CommandKind, CommandStatus, UserCommand, UserCommandInbox};

fn test_directory() -> tempfile::TempDir {
    tempfile::tempdir().expect("temporary test directory creates")
}

fn goal() -> GoalContract {
    GoalContract::new(
        "goal-interrupt",
        "project-interrupt",
        "test interrupt",
        vec![],
        vec![],
        vec!["done".into()],
    )
}

fn steering_command(
    controller: &ReadOnlyLoopController,
    directory: &std::path::Path,
    command_id: &str,
    kind: CommandKind,
) -> UserCommand {
    let started = EventLog::open(directory.join("events.jsonl"))
        .expect("log reopens")
        .read_all()
        .expect("events read")
        .into_iter()
        .find(|event| event.event_type == "loop_started")
        .expect("controller startup records its identity");
    assert_eq!(
        started.task_run_id.as_deref(),
        Some(controller.task_run().task_run_id.as_str())
    );
    UserCommand::new(
        command_id,
        "operator-test",
        started.project_id.expect("startup has project identity"),
        started.goal_id.expect("startup has goal identity"),
        controller.task_run().task_run_id.clone(),
        started.loop_run_id.expect("startup has loop identity"),
        kind,
        format!("corr-{}", command_id),
        serde_json::json!({"reason": "test"}),
    )
    .expect("command valid")
}

#[test]
fn pause_command_responds_within_safe_boundary() {
    let dir = test_directory();

    let inbox = UserCommandInbox::open(dir.path().join("commands.jsonl"));
    let mut controller = ReadOnlyLoopController::start(
        EventLog::open(dir.path().join("events.jsonl")).expect("log opens"),
        goal(),
        "loop-interrupt",
        "task-interrupt",
        LoopBudget::new(10, 0, 0, 1, 0, 10, 1, 1),
    )
    .expect("controller starts")
    .with_command_inbox(inbox.clone());

    controller
        .run_observation("step-1", "observation")
        .expect("step runs");

    let pause_cmd = steering_command(&controller, dir.path(), "pause-1", CommandKind::Pause);
    let budget_before_pause = controller.budget().clone();

    let submit_start = Instant::now();
    inbox.submit(pause_cmd).expect("command submits");

    assert!(matches!(
        controller.run_observation("step-2", "observation"),
        Err(LoopControllerError::Paused)
    ));

    let response_time = submit_start.elapsed();

    println!("Pause 响应时间: {:?}", response_time);

    assert!(
        response_time < Duration::from_secs(1),
        "Pause 响应时间 {:?} 超过 1 秒",
        response_time
    );

    assert_eq!(controller.status(), LoopStatus::Paused);
    assert_eq!(controller.budget(), &budget_before_pause);
    assert_eq!(
        inbox.latest("pause-1").unwrap().status,
        CommandStatus::Applied
    );
    let events = EventLog::open(dir.path().join("events.jsonl"))
        .unwrap()
        .read_all()
        .unwrap();
    assert!(events.iter().any(|event| {
        event.event_type == "loop_paused" && event.payload["command_id"] == "pause-1"
    }));
    assert!(!events
        .iter()
        .any(|event| event.step_id.as_deref() == Some("step-2")));
}

#[test]
fn cancel_command_stops_execution_at_next_safe_boundary() {
    let dir = test_directory();

    let inbox = UserCommandInbox::open(dir.path().join("commands.jsonl"));
    let mut controller = ReadOnlyLoopController::start(
        EventLog::open(dir.path().join("events.jsonl")).expect("log opens"),
        goal(),
        "loop-cancel",
        "task-cancel",
        LoopBudget::new(10, 0, 0, 1, 0, 10, 1, 1),
    )
    .expect("controller starts")
    .with_command_inbox(inbox.clone());

    controller
        .run_observation("step-1", "obs 1")
        .expect("step 1 runs");

    let cancel_cmd = steering_command(&controller, dir.path(), "cancel-1", CommandKind::Cancel);
    let budget_before_cancel = controller.budget().clone();
    inbox.submit(cancel_cmd).expect("cancel submits");

    assert!(matches!(
        controller.run_observation("step-2", "obs 2"),
        Err(LoopControllerError::Terminal)
    ));
    assert_eq!(controller.status(), LoopStatus::Cancelled);
    assert_eq!(controller.budget(), &budget_before_cancel);
    assert_eq!(
        inbox.latest("cancel-1").unwrap().status,
        CommandStatus::Applied
    );
    assert!(matches!(
        controller.run_observation("step-3", "still cancelled"),
        Err(LoopControllerError::Terminal)
    ));

    let events = EventLog::open(dir.path().join("events.jsonl"))
        .expect("log reopens")
        .read_all()
        .expect("events read");

    assert!(events.iter().any(|event| {
        event.event_type == "loop_cancelled" && event.payload["command_id"] == "cancel-1"
    }));
    assert!(!events
        .iter()
        .any(|event| { matches!(event.step_id.as_deref(), Some("step-2" | "step-3")) }));
}

#[test]
fn resume_command_restores_execution() {
    let dir = test_directory();

    let inbox = UserCommandInbox::open(dir.path().join("commands.jsonl"));
    let mut controller = ReadOnlyLoopController::start(
        EventLog::open(dir.path().join("events.jsonl")).expect("log opens"),
        goal(),
        "loop-resume",
        "task-resume",
        LoopBudget::new(10, 0, 0, 1, 0, 10, 1, 1),
    )
    .expect("controller starts")
    .with_command_inbox(inbox.clone());

    controller
        .run_observation("step-1", "obs 1")
        .expect("step 1 runs");

    let budget_before_pause = controller.budget().clone();
    let pause_cmd = steering_command(&controller, dir.path(), "pause-2", CommandKind::Pause);
    inbox.submit(pause_cmd).expect("pause submits");
    assert!(matches!(
        controller.run_observation("step-paused", "must not execute"),
        Err(LoopControllerError::Paused)
    ));
    assert_eq!(controller.status(), LoopStatus::Paused);
    assert_eq!(controller.budget(), &budget_before_pause);
    assert_eq!(
        inbox.latest("pause-2").unwrap().status,
        CommandStatus::Applied
    );

    let resume_cmd = steering_command(&controller, dir.path(), "resume-1", CommandKind::Resume);
    let resume_time = Instant::now();
    inbox.submit(resume_cmd).expect("resume submits");

    controller
        .run_observation("step-2", "obs 2")
        .expect("resume allows the next step");
    let response_time = resume_time.elapsed();

    println!("Resume 响应时间: {:?}", response_time);

    assert!(
        response_time < Duration::from_secs(1),
        "Resume 响应时间 {:?} 超过 1 秒",
        response_time
    );

    assert_eq!(controller.status(), LoopStatus::Running);
    assert_eq!(
        controller.budget().steps_used,
        budget_before_pause.steps_used + 1
    );
    assert_eq!(
        inbox.latest("resume-1").unwrap().status,
        CommandStatus::Applied
    );
    let events = EventLog::open(dir.path().join("events.jsonl"))
        .unwrap()
        .read_all()
        .unwrap();
    assert!(events.iter().any(|event| {
        event.event_type == "loop_resumed" && event.payload["command_id"] == "resume-1"
    }));
    assert!(events.iter().any(|event| {
        event.event_type == "step_completed" && event.step_id.as_deref() == Some("step-2")
    }));
    assert!(!events
        .iter()
        .any(|event| event.step_id.as_deref() == Some("step-paused")));
}

#[test]
fn unsupported_takeover_is_rejected_without_claiming_handoff() {
    let dir = test_directory();

    let inbox = UserCommandInbox::open(dir.path().join("commands.jsonl"));
    let mut controller = ReadOnlyLoopController::start(
        EventLog::open(dir.path().join("events.jsonl")).expect("log opens"),
        goal(),
        "loop-takeover",
        "task-takeover",
        LoopBudget::new(10, 0, 0, 1, 0, 10, 1, 1),
    )
    .expect("controller starts")
    .with_command_inbox(inbox.clone());

    controller
        .run_observation("step-1", "obs 1")
        .expect("step 1 runs");

    let attempt_before = controller.current_attempt_id().unwrap().to_owned();
    let budget_before = controller.budget().clone();
    let takeover_cmd =
        steering_command(&controller, dir.path(), "takeover-1", CommandKind::Takeover);
    inbox.submit(takeover_cmd).expect("takeover submits");

    controller
        .run_observation("step-2", "obs 2")
        .expect("unsupported takeover does not stop normal action");
    assert_eq!(
        inbox.latest("takeover-1").unwrap().status,
        CommandStatus::Rejected
    );
    assert_eq!(controller.status(), LoopStatus::Running);
    assert_eq!(
        controller.current_attempt_id(),
        Some(attempt_before.as_str())
    );
    let mut expected_budget = budget_before;
    expected_budget.consume_step().unwrap();
    assert_eq!(controller.budget(), &expected_budget);

    let events = EventLog::open(dir.path().join("events.jsonl"))
        .expect("log reopens")
        .read_all()
        .expect("events read");

    assert!(events.iter().any(|event| {
        event.event_type == "user_command_rejected"
            && event.payload["command_id"] == "takeover-1"
            && event.payload["reason"] == "command kind is not implemented at this safe boundary"
    }));
    assert!(!events.iter().any(|event| {
        event.event_type == "user_command_applied" && event.payload["command_id"] == "takeover-1"
    }));
}
