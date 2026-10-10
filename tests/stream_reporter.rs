use std::io;

use steerbox::stream_reporter::{
    ProgressEvent, ProgressSink, ProgressStatus, ProgressSubject, StreamReporter,
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
