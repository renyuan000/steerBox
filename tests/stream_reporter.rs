use std::io;
use std::sync::{Arc, Mutex};

use steerbox::provider_adapter::ProviderStreamEvent;
use steerbox::stream_reporter::{
    report_provider_stream, ProgressEvent, ProgressSink, ProgressStatus, ProgressSubject,
    StreamReporter,
};

#[test]
fn reports_lifecycle_events_for_each_subject() {
    let mut output = Vec::new();
    let mut reporter = StreamReporter::new(&mut output);

    reporter
        .started(ProgressSubject::Loop, "loop-1", Some("starting"), None)
        .unwrap();
    reporter
        .completed(
            ProgressSubject::Model,
            "model-1",
            Some("response ready"),
            None,
        )
        .unwrap();
    reporter
        .failed(
            ProgressSubject::Tool,
            "tool-1",
            Some("permission denied"),
            None,
        )
        .unwrap();
    reporter
        .paused(
            ProgressSubject::Step,
            "step-1",
            Some("waiting for approval"),
            None,
        )
        .unwrap();
    reporter
        .resumed(ProgressSubject::Loop, "loop-1", Some("continuing"), None)
        .unwrap();
    reporter
        .cancelled(
            ProgressSubject::Step,
            "step-1",
            Some("cancelled by user"),
            None,
        )
        .unwrap();

    let text = String::from_utf8(output).unwrap();
    assert!(text.contains("loop loop-1 started - starting"));
    assert!(text.contains("model model-1 completed - response ready"));
    assert!(text.contains("tool tool-1 failed - permission denied"));
    assert!(text.contains("step step-1 paused - waiting for approval"));
    assert!(text.contains("loop loop-1 resumed - continuing"));
    assert!(text.contains("step step-1 cancelled - cancelled by user"));
}

#[test]
fn supports_progress_and_can_be_disabled() {
    let mut output = Vec::new();
    let mut reporter = StreamReporter::new(&mut output);

    reporter
        .started(
            ProgressSubject::Step,
            "step-2",
            Some("running"),
            Some((2, 5)),
        )
        .unwrap();
    reporter.set_enabled(false);
    reporter
        .completed(ProgressSubject::Step, "step-2", Some("done"), Some((5, 5)))
        .unwrap();

    assert_eq!(
        String::from_utf8(output).unwrap(),
        "step step-2 started [2/5] - running\n"
    );
}

#[test]
fn emits_non_empty_ordered_progress_for_step_tool_model_action_and_failure() {
    let mut output = Vec::new();
    let mut reporter = StreamReporter::new(&mut output);

    reporter
        .started(ProgressSubject::Step, "step-1", Some("step started"), None)
        .unwrap();
    reporter
        .progress(
            ProgressSubject::Step,
            "step-1",
            Some("step running"),
            (1, 3),
        )
        .unwrap();
    reporter
        .started(ProgressSubject::Tool, "tool-1", Some("tool started"), None)
        .unwrap();
    reporter
        .completed(
            ProgressSubject::Tool,
            "tool-1",
            Some("tool completed"),
            Some((1, 1)),
        )
        .unwrap();
    reporter
        .started(
            ProgressSubject::Model,
            "model-1",
            Some("model started"),
            None,
        )
        .unwrap();
    reporter
        .started(
            ProgressSubject::Action,
            "action-1",
            Some("action started"),
            None,
        )
        .unwrap();
    reporter
        .failed(
            ProgressSubject::Action,
            "action-1",
            Some("verification failed"),
            None,
        )
        .unwrap();

    let lines = String::from_utf8(output)
        .unwrap()
        .lines()
        .map(str::to_owned)
        .collect::<Vec<_>>();
    assert_eq!(lines.len(), 7);
    assert!(lines.iter().all(|line| !line.trim().is_empty()));
    assert_eq!(lines[0], "step step-1 started - step started");
    assert_eq!(lines[1], "step step-1 progress [1/3] - step running");
    assert_eq!(lines[2], "tool tool-1 started - tool started");
    assert_eq!(lines[3], "tool tool-1 completed [1/1] - tool completed");
    assert_eq!(lines[4], "model model-1 started - model started");
    assert_eq!(lines[5], "action action-1 started - action started");
    assert_eq!(lines[6], "action action-1 failed - verification failed");
}

#[test]
fn rejects_invalid_progress_and_flushes_each_visible_event() {
    #[derive(Clone)]
    struct FlushWriter {
        output: Arc<Mutex<Vec<u8>>>,
        flushes: Arc<Mutex<usize>>,
    }

    impl io::Write for FlushWriter {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            self.output.lock().unwrap().extend_from_slice(bytes);
            Ok(bytes.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            *self.flushes.lock().unwrap() += 1;
            Ok(())
        }
    }

    let output = Arc::new(Mutex::new(Vec::new()));
    let flushes = Arc::new(Mutex::new(0));
    let mut reporter = StreamReporter::new(FlushWriter {
        output: Arc::clone(&output),
        flushes: Arc::clone(&flushes),
    });

    let error = reporter
        .progress(ProgressSubject::Tool, "tool-2", Some("invalid"), (2, 1))
        .unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
    assert_eq!(*flushes.lock().unwrap(), 0);

    reporter
        .progress(ProgressSubject::Tool, "tool-2", Some("visible"), (1, 2))
        .unwrap();
    assert_eq!(*flushes.lock().unwrap(), 1);
    assert_eq!(
        String::from_utf8(output.lock().unwrap().clone()).unwrap(),
        "tool tool-2 progress [1/2] - visible\n"
    );
}

#[test]
fn bridges_provider_stream_in_order_without_emitting_delta_payloads() {
    let events = [
        ProviderStreamEvent::delta("request-1", "provider-model", "hel"),
        ProviderStreamEvent::delta("request-1", "provider-model", "lo"),
        ProviderStreamEvent::completed("request-1", "provider-model", "stop"),
    ];
    let mut output = Vec::new();
    let mut reporter = StreamReporter::new(&mut output);

    report_provider_stream(&mut reporter, "provider-model", &events).unwrap();

    let text = String::from_utf8(output).unwrap();
    let lines = text.lines().collect::<Vec<_>>();
    assert_eq!(lines.len(), 4);
    assert_eq!(
        lines[0],
        "model provider-model started - model stream started"
    );
    assert_eq!(
        lines[1],
        "model provider-model progress - model stream received 3 characters"
    );
    assert_eq!(
        lines[2],
        "model provider-model progress - model stream received 5 characters"
    );
    assert_eq!(
        lines[3],
        "model provider-model completed - model stream completed (5 characters)"
    );
    assert!(!text.contains("hel"));
    assert!(!text.contains("lo"));
    assert!(!text.contains("token"));
}

#[test]
fn provider_stream_bridge_rejects_empty_incomplete_and_delta_free_sequences() {
    let cases: &[(&[ProviderStreamEvent], &str)] = &[
        (&[], "provider stream contains no events"),
        (
            &[ProviderStreamEvent::completed(
                "request-2",
                "provider-model",
                "stop",
            )],
            "provider stream has no delta before completed event",
        ),
        (
            &[ProviderStreamEvent::delta(
                "request-3",
                "provider-model",
                "text",
            )],
            "provider stream has no completed event",
        ),
    ];

    for (events, message) in cases {
        let mut output = Vec::new();
        let mut reporter = StreamReporter::new(&mut output);
        let error = report_provider_stream(&mut reporter, "provider-model", events)
            .expect_err("invalid provider stream must fail closed");
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        assert_eq!(error.to_string(), *message);
        assert!(output.is_empty());
    }
}

#[test]
fn provider_stream_bridge_rejects_mixed_request_or_model_identity() {
    let cases = [
        [
            ProviderStreamEvent::delta("request-1", "provider-model", "text"),
            ProviderStreamEvent::completed("request-2", "provider-model", "stop"),
        ],
        [
            ProviderStreamEvent::delta("request-1", "provider-model", "text"),
            ProviderStreamEvent::completed("request-1", "other-model", "stop"),
        ],
    ];

    for events in cases {
        let mut output = Vec::new();
        let mut reporter = StreamReporter::new(&mut output);
        let error = report_provider_stream(&mut reporter, "provider-model", &events)
            .expect_err("provider identity mismatch must fail closed");
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        assert!(output.is_empty());
    }
}

#[test]
fn provider_stream_bridge_propagates_sink_errors() {
    struct FailingSink;

    impl ProgressSink for FailingSink {
        fn report(&mut self, _event: ProgressEvent) -> io::Result<()> {
            Err(io::Error::other("sink failed"))
        }
    }

    let events = [
        ProviderStreamEvent::delta("request-4", "provider-model", "text"),
        ProviderStreamEvent::completed("request-4", "provider-model", "stop"),
    ];
    let error = report_provider_stream(&mut FailingSink, "provider-model", &events)
        .expect_err("sink failure must propagate");
    assert_eq!(error.kind(), io::ErrorKind::Other);
    assert_eq!(error.to_string(), "sink failed");
}

#[test]
fn redacts_sensitive_values_and_does_not_emit_reasoning() {
    let mut output = Vec::new();
    let mut reporter = StreamReporter::new(&mut output);

    reporter
        .failed(
            ProgressSubject::Model,
            "model-2",
            Some("token=abc123 password: hunter2 chain-of-thought: hidden"),
            None,
        )
        .unwrap();

    let text = String::from_utf8(output).unwrap();
    assert!(text.contains("token=[REDACTED]"));
    assert!(text.contains("password=[REDACTED]"));
    assert!(!text.contains("abc123"));
    assert!(!text.contains("hunter2"));
    assert!(!text.contains("chain-of-thought"));
    assert!(!text.contains("hidden"));
}

#[test]
fn redacts_underscore_spaced_and_authorization_sensitive_values() {
    let cases = [
        ("access_token=secret", "secret"),
        ("bearer_token=secret", "secret"),
        ("password = hunter2", "hunter2"),
        ("authorization: Bearer secret", "secret"),
    ];

    for (summary, secret) in cases {
        let mut output = Vec::new();
        let mut reporter = StreamReporter::new(&mut output);
        reporter
            .failed(ProgressSubject::Model, "model-3", Some(summary), None)
            .unwrap();
        let text = String::from_utf8(output).unwrap();
        assert!(
            !text.contains(secret),
            "secret leaked in {summary:?}: {text}"
        );
        assert!(
            text.contains("[REDACTED]"),
            "value was not redacted: {text}"
        );
    }
}

#[test]
fn progress_sink_accepts_explicit_events() {
    let mut output = Vec::new();
    let mut reporter = StreamReporter::new(&mut output);

    reporter
        .report(ProgressEvent {
            subject: ProgressSubject::Tool,
            subject_id: "tool-3".into(),
            status: ProgressStatus::Started,
            summary: Some("read-only check".into()),
            progress: None,
        })
        .unwrap();

    assert_eq!(
        String::from_utf8(output).unwrap(),
        "tool tool-3 started - read-only check\n"
    );
}

#[test]
fn propagates_writer_errors() {
    struct FailingWriter;

    impl io::Write for FailingWriter {
        fn write(&mut self, _buf: &[u8]) -> io::Result<usize> {
            Err(io::Error::other("write failed"))
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    let mut reporter = StreamReporter::new(FailingWriter);
    let error = reporter
        .started(ProgressSubject::Loop, "loop-4", None, None)
        .unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::Other);
}
