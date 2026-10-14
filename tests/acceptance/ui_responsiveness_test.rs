// 本地 controller progress smoke 测试，不是 UI 或模型首字节性能验收。
//
// 测试目标：
// 1. 本地 LoopStarted 进度在 500ms 内到达内存 sink
// 2. 连续本地 observation 的进度间隔 < 2s
// 3. 每个 observation 发出身份正确且有序的 Started/Completed

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use steerbox::domain::{GoalContract, LoopBudget, LoopStatus};
use steerbox::event_log::EventLog;
use steerbox::loop_controller::ReadOnlyLoopController;
use steerbox::stream_reporter::{ProgressEvent, ProgressSink, ProgressStatus, ProgressSubject};

fn test_directory() -> tempfile::TempDir {
    tempfile::tempdir().expect("temporary test directory creates")
}

struct RecordingSink(Arc<Mutex<Vec<(Instant, ProgressEvent)>>>);

impl ProgressSink for RecordingSink {
    fn report(&mut self, event: ProgressEvent) -> std::io::Result<()> {
        self.0.lock().unwrap().push((Instant::now(), event));
        Ok(())
    }
}

#[test]
fn local_loop_started_progress_arrives_within_500ms() {
    let dir = test_directory();

    let recorded = Arc::new(Mutex::new(Vec::new()));
    let start = Instant::now();

    let controller = ReadOnlyLoopController::start_with_progress_sink(
        EventLog::open(dir.path().join("events.jsonl")).expect("log opens"),
        GoalContract::new(
            "goal-ui-test",
            "project-ui-test",
            "test responsiveness",
            vec![],
            vec![],
            vec!["done".into()],
        ),
        "loop-ui-test",
        "task-ui-test",
        LoopBudget::new(5, 0, 0, 1, 0, 5, 1, 1),
        RecordingSink(recorded.clone()),
    )
    .expect("controller starts");

    assert_eq!(controller.status(), LoopStatus::Running);
    let events = recorded.lock().unwrap();
    let (reported_at, _) = events
        .iter()
        .find(|(_, event)| {
            event.subject == ProgressSubject::Loop
                && event.subject_id == "loop-ui-test"
                && event.status == ProgressStatus::Started
        })
        .expect("matching loop-start progress is observable");
    let progress_latency = reported_at.duration_since(start);
    assert!(
        progress_latency < Duration::from_millis(500),
        "本地启动进度延迟 {:?} 超过 500ms",
        progress_latency
    );
}

#[test]
fn local_observation_progress_interval_is_under_2_seconds() {
    let dir = test_directory();

    let recorded = Arc::new(Mutex::new(Vec::new()));
    let mut controller = ReadOnlyLoopController::start_with_progress_sink(
        EventLog::open(dir.path().join("events.jsonl")).expect("log opens"),
        GoalContract::new(
            "goal-step-test",
            "project-step-test",
            "test step interval",
            vec![],
            vec![],
            vec!["done".into()],
        ),
        "loop-step-test",
        "task-step-test",
        LoopBudget::new(5, 0, 0, 1, 0, 5, 1, 1),
        RecordingSink(recorded.clone()),
    )
    .expect("controller starts");

    // 运行第一个观察步骤
    controller
        .run_observation("step-1", "first observation")
        .expect("step 1 runs");

    // 运行第二个观察步骤
    controller
        .run_observation("step-2", "second observation")
        .expect("step 2 runs");

    let events = recorded.lock().unwrap();
    let first_completed = events
        .iter()
        .find(|(_, event)| {
            event.subject == ProgressSubject::Step
                && event.subject_id == "step-1"
                && event.status == ProgressStatus::Completed
        })
        .expect("first step completion is observable")
        .0;
    let second_started = events
        .iter()
        .find(|(_, event)| {
            event.subject == ProgressSubject::Step
                && event.subject_id == "step-2"
                && event.status == ProgressStatus::Started
        })
        .expect("second step start is observable")
        .0;
    let step_interval = second_started
        .checked_duration_since(first_completed)
        .expect("second step starts after first step completes");

    println!("步骤间隔: {:?}", step_interval);

    // 验收标准：< 2s
    assert!(
        step_interval < Duration::from_secs(2),
        "步骤间隔 {:?} 超过 2 秒",
        step_interval
    );
}

#[test]
fn each_local_observation_emits_matching_started_and_completed_progress() {
    let dir = test_directory();

    let recorded = Arc::new(Mutex::new(Vec::new()));
    let mut controller = ReadOnlyLoopController::start_with_progress_sink(
        EventLog::open(dir.path().join("events.jsonl")).expect("log opens"),
        GoalContract::new(
            "goal-observable",
            "project-observable",
            "test observable output",
            vec![],
            vec![],
            vec!["done".into()],
        ),
        "loop-observable",
        "task-observable",
        LoopBudget::new(5, 0, 0, 1, 0, 5, 1, 1),
        RecordingSink(recorded.clone()),
    )
    .expect("controller starts");

    let initial_count = recorded.lock().unwrap().len();

    // 运行 3 个观察步骤
    for i in 1..=3 {
        let before = recorded.lock().unwrap().len();

        controller
            .run_observation(format!("step-{}", i), format!("observation {}", i))
            .expect("step runs");

        let events = recorded.lock().unwrap();
        let step_id = format!("step-{}", i);
        assert_eq!(
            events[before..]
                .iter()
                .map(|(_, event)| (event.subject, event.subject_id.as_str(), event.status))
                .collect::<Vec<_>>(),
            vec![
                (
                    ProgressSubject::Step,
                    step_id.as_str(),
                    ProgressStatus::Started
                ),
                (
                    ProgressSubject::Step,
                    step_id.as_str(),
                    ProgressStatus::Completed
                ),
            ]
        );
    }

    let total_events = recorded.lock().unwrap().len() - initial_count;
    assert_eq!(total_events, 6);
}

#[test]
fn three_local_observations_and_completion_finish_within_2_seconds() {
    let dir = test_directory();

    let mut controller = ReadOnlyLoopController::start(
        EventLog::open(dir.path().join("events.jsonl")).expect("log opens"),
        GoalContract::new(
            "goal-total-time",
            "project-total-time",
            "test total execution time",
            vec![],
            vec![],
            vec!["done".into()],
        ),
        "loop-total-time",
        "task-total-time",
        LoopBudget::new(5, 0, 0, 1, 0, 5, 1, 1),
    )
    .expect("controller starts");

    let start = Instant::now();

    // 运行 3 个步骤并完成
    for i in 1..=3 {
        controller
            .run_observation(format!("step-{}", i), format!("observation {}", i))
            .expect("step runs");
    }

    controller.complete().expect("loop completes");
    assert_eq!(controller.status(), LoopStatus::Completed);
    assert_eq!(controller.budget().steps_used, 3);

    let total_time = start.elapsed();

    println!("总执行时间: {:?}", total_time);

    // 验收标准：3 步应在 2 秒内完成
    assert!(
        total_time < Duration::from_secs(2),
        "总执行时间 {:?} 超过 2 秒",
        total_time
    );
}
