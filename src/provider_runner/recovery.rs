//! Explicit pure-observation recovery. All sends still cross the controller.
use super::*;
use crate::anthropic_adapter::{AnthropicHeaders, AnthropicTransportError};
use crate::http_transport::HttpAnthropicTransport;
use crate::provider_recovery::RecoveryGap;
use serde_json::json;
use std::time::Instant;

/// At most two calls, one controller, no tools. Existing single-call APIs stay unchanged.
pub struct ForegroundRecoveryProviderRun {
    inner: ForegroundProviderRun,
    config: RuntimeConfig,
    total_timeout: Duration,
}

impl ForegroundProviderRun {
    /// Enable one audited retry/repair for pure observation, not a tool or write run.
    /// The total deadline must be 1..=120_000 ms. Does not access a provider.
    pub fn prepare_recovering(
        config: &RuntimeConfig,
        request: ProviderRunRequest,
        total_timeout_ms: u64,
    ) -> Result<ForegroundRecoveryProviderRun, ProviderRunError> {
        if !(1..=120_000).contains(&total_timeout_ms) {
            return Err(ProviderRunError::InvalidRequest);
        }
        let encoded =
            serde_json::to_string(config).map_err(|_| ProviderRunError::InvalidRequest)?;
        let config = RuntimeConfig::from_json_str(&encoded)?;
        let inner = Self::prepare(&config, request)?;
        Ok(ForegroundRecoveryProviderRun {
            inner,
            config,
            total_timeout: Duration::from_millis(total_timeout_ms),
        })
    }
}

impl ForegroundRecoveryProviderRun {
    pub fn run_with_resolver<R: CredentialResolver, S: ProgressSink + 'static>(
        self,
        resolver: R,
        events: EventLog,
        model_calls: ModelCallStore,
        sink: S,
    ) -> Result<ProviderRunOutcome, ProviderRunError> {
        match self.inner.profile.endpoint_protocol.as_str() {
            OPENAI_COMPATIBLE_CHAT_PROTOCOL | OPENAI_RESPONSES_PROTOCOL => {
                let transport = HttpProviderTransport::new(resolver)
                    .map_err(|_| ProviderRunError::TransportBuild)?;
                self.run_with_transport(transport, events, model_calls, sink)
            }
            ANTHROPIC_MESSAGES_PROTOCOL => {
                let transport = HttpAnthropicTransport::new(resolver)
                    .map_err(|_| ProviderRunError::TransportBuild)?;
                self.run_anthropic_with_transport(transport, events, model_calls, sink)
            }
            _ => Err(ProviderRunError::UnsupportedProtocol),
        }
    }

    pub fn run_with_transport<T: ProviderTransport, S: ProgressSink + 'static>(
        self,
        transport: T,
        events: EventLog,
        model_calls: ModelCallStore,
        sink: S,
    ) -> Result<ProviderRunOutcome, ProviderRunError> {
        if !matches!(
            self.inner.profile.endpoint_protocol.as_str(),
            OPENAI_COMPATIBLE_CHAT_PROTOCOL | OPENAI_RESPONSES_PROTOCOL
        ) {
            return Err(ProviderRunError::UnsupportedProtocol);
        }
        let transport = Rc::new(transport);
        self.run(events, model_calls, sink, |profile, deadline| {
            Ok(profile.build_openai_adapter(DeadlineTransport {
                inner: transport.clone(),
                deadline,
            })?)
        })
    }

    pub fn run_anthropic_with_transport<T: AnthropicTransport, S: ProgressSink + 'static>(
        self,
        transport: T,
        events: EventLog,
        model_calls: ModelCallStore,
        sink: S,
    ) -> Result<ProviderRunOutcome, ProviderRunError> {
        if self.inner.profile.endpoint_protocol != ANTHROPIC_MESSAGES_PROTOCOL {
            return Err(ProviderRunError::UnsupportedProtocol);
        }
        let transport = Rc::new(transport);
        self.run(events, model_calls, sink, |profile, deadline| {
            Ok(AnthropicAdapter::new(
                &profile.provider,
                &profile.endpoint,
                DeadlineTransport {
                    inner: transport.clone(),
                    deadline,
                },
                profile.auth_ref.clone(),
            )
            .with_endpoint_mode(profile.endpoint_mode)
            .with_model_id(&profile.model_id))
        })
    }

    fn run<A: ResolvedModelAdapter, S: ProgressSink + 'static, F>(
        self,
        events: EventLog,
        model_calls: ModelCallStore,
        sink: S,
        mut build: F,
    ) -> Result<ProviderRunOutcome, ProviderRunError>
    where
        F: FnMut(&ModelProfileConfig, Instant) -> Result<A, ProviderRunError>,
    {
        let deadline = Instant::now() + self.total_timeout;
        let Self { inner, config, .. } = self;
        reject_reused_run_identity(&events, &model_calls, &inner.request)?;
        let tool_calls = ToolCallRecordStore::open(tool_call_store_path(events.path()))?;
        let request = inner.request.clone();
        let goal = GoalContract::new(
            &request.goal_id,
            &request.project_id,
            &request.objective,
            Vec::new(),
            Vec::new(),
            vec!["model_result_recorded".into()],
        );
        let mut controller = ReadOnlyLoopController::start_with_progress_sink(
            events,
            goal,
            &request.loop_run_id,
            &request.task_run_id,
            LoopBudget::new(2, 0, 2, 2, 0, 1, 1, 1),
            sink,
        )?
        .with_model_call_store(model_calls)
        .with_tool_call_store(tool_calls);
        let result = (|| {
            configure_reported_token_budget(&mut controller, inner.reported_token_limit)?;
            let mut bound = inner;
            let mut gap = None;
            for index in 0..2 {
                if deadline
                    .saturating_duration_since(Instant::now())
                    .as_millis()
                    == 0
                {
                    return Err(ProviderRunError::RecoveryDeadlineExceeded);
                }
                let adapter = build(&bound.profile, deadline)?;
                let step_id = format!("step-{}", index + 1);
                let model_request = ModelRequest {
                    model_id: request.model_id.clone(),
                    goal_id: request.goal_id.clone(),
                    loop_run_id: request.loop_run_id.clone(),
                    step_id: step_id.clone(),
                    objective: request.objective.clone(),
                    observation: bound.request.observation.clone(),
                };
                let result = if let Some(gap) = gap {
                    controller.run_resolved_observation_retry(
                        &adapter,
                        model_request,
                        &bound.prompt,
                        &bound.route,
                        step_id,
                        gap,
                    )
                } else {
                    controller.run_resolved_model_step(
                        &adapter,
                        model_request,
                        &bound.prompt,
                        &bound.route,
                        step_id,
                    )
                };
                match result {
                    Ok(decision) => {
                        return Ok(ProviderRunOutcome {
                            decision,
                            loop_status: controller.status(),
                            reported_token_budget: controller
                                .task_run()
                                .reported_token_budget
                                .clone(),
                        })
                    }
                    Err(error) => {
                        let next_gap = match &error {
                            LoopControllerError::ModelAdapter(error) => {
                                RecoveryGap::from_error(error)
                            }
                            _ => None,
                        };
                        if index == 1 || next_gap.is_none() {
                            return Err(error.into());
                        }
                        // A strict reported-token ledger may block after unknown usage.
                        // Never reset it, substitute zero, or create a fresh controller.
                        if let Some(budget) = controller.task_run().reported_token_budget() {
                            budget
                                .check_send(&format!("call-{}-step-2", request.loop_run_id))
                                .map_err(LoopControllerError::from)?;
                        }
                        let next_gap = next_gap.expect("checked above");
                        if next_gap == RecoveryGap::TransientProvider {
                            let delay = Duration::from_millis(200);
                            if deadline.saturating_duration_since(Instant::now()) <= delay {
                                return Err(ProviderRunError::RecoveryDeadlineExceeded);
                            }
                            std::thread::sleep(delay);
                        }
                        let mut repaired = request.clone();
                        if next_gap != RecoveryGap::TransientProvider {
                            repaired.observation = json!({"original_observation":request.observation,
                                "recovery_gap":next_gap.code(),
                                "required_result":"Return one complete decision envelope, without actions. If unsafe or human authorization is needed, return request_human. Do not repeat or invent tool execution."}).to_string();
                        }
                        let mut route = bound.route.clone();
                        route.route_decision_id = format!("{}:recovery:1", route.route_decision_id);
                        bound = ForegroundProviderRun::prepare_internal(
                            &config,
                            repaired,
                            None,
                            Some(&route),
                            None,
                        )?;
                        gap = Some(next_gap);
                    }
                }
            }
            unreachable!("two attempts either return a result or an error")
        })();
        if result.is_err() {
            controller.close_failed_foreground_run()?;
        }
        result
    }
}

struct DeadlineTransport<T> {
    inner: Rc<T>,
    deadline: Instant,
}

impl<T: ProviderTransport> ProviderTransport for DeadlineTransport<T> {
    fn send(
        &self,
        endpoint: &str,
        auth_ref: Option<&str>,
        body: &str,
        timeout: Duration,
        streaming: bool,
    ) -> Result<TransportResponse, TransportError> {
        let remaining = self.deadline.saturating_duration_since(Instant::now());
        if remaining.as_millis() == 0 {
            return Err(TransportError::Timeout);
        }
        let result = self
            .inner
            .send(endpoint, auth_ref, body, timeout.min(remaining), streaming);
        // Preserve definitive safety/HTTP errors even if the transport returns
        // late. The host deadline must reject late success, not erase evidence.
        if matches!(&result, Err(error) if !matches!(error, TransportError::Timeout | TransportError::Network))
            || matches!(&result, Ok(response) if !(200..300).contains(&response.status_code))
        {
            return result;
        }
        if Instant::now() >= self.deadline {
            return Err(TransportError::Timeout);
        }
        result
    }
}

impl<T: AnthropicTransport> AnthropicTransport for DeadlineTransport<T> {
    fn send(
        &self,
        endpoint: &str,
        headers: &AnthropicHeaders,
        body: &str,
        timeout: Duration,
        streaming: bool,
    ) -> Result<TransportResponse, AnthropicTransportError> {
        let remaining = self.deadline.saturating_duration_since(Instant::now());
        if remaining.as_millis() == 0 {
            return Err(AnthropicTransportError::Timeout);
        }
        let result = self
            .inner
            .send(endpoint, headers, body, timeout.min(remaining), streaming);
        if matches!(&result, Err(error) if !matches!(error, AnthropicTransportError::Timeout | AnthropicTransportError::Network))
            || matches!(&result, Ok(response) if !(200..300).contains(&response.status_code))
        {
            return result;
        }
        if Instant::now() >= self.deadline {
            return Err(AnthropicTransportError::Timeout);
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    struct Probe {
        timeouts: RefCell<Vec<Duration>>,
        late: bool,
    }

    impl ProviderTransport for Probe {
        fn send(
            &self,
            _: &str,
            _: Option<&str>,
            _: &str,
            timeout: Duration,
            _: bool,
        ) -> Result<TransportResponse, TransportError> {
            self.timeouts.borrow_mut().push(timeout);
            if self.late {
                std::thread::sleep(timeout + Duration::from_millis(5));
            }
            Ok(TransportResponse::body(200, "synthetic success"))
        }
    }

    impl AnthropicTransport for Probe {
        fn send(
            &self,
            endpoint: &str,
            _: &AnthropicHeaders,
            body: &str,
            timeout: Duration,
            streaming: bool,
        ) -> Result<TransportResponse, AnthropicTransportError> {
            ProviderTransport::send(self, endpoint, None, body, timeout, streaming)
                .map_err(|_| AnthropicTransportError::Timeout)
        }
    }

    fn send(probe: &DeadlineTransport<Probe>, messages: bool) -> bool {
        if messages {
            let headers = AnthropicHeaders {
                api_key_ref: None,
                anthropic_version: "2023-06-01".into(),
            };
            AnthropicTransport::send(
                probe,
                "https://example.test",
                &headers,
                "{}",
                Duration::from_secs(30),
                false,
            )
            .is_ok()
        } else {
            ProviderTransport::send(
                probe,
                "https://example.test",
                None,
                "{}",
                Duration::from_secs(30),
                false,
            )
            .is_ok()
        }
    }

    #[test]
    fn subsequent_transport_timeout_is_clipped_to_the_remaining_shared_deadline() {
        for messages in [false, true] {
            let inner = Rc::new(Probe {
                timeouts: RefCell::new(Vec::new()),
                late: false,
            });
            let probe = DeadlineTransport {
                inner: inner.clone(),
                deadline: Instant::now() + Duration::from_secs(10),
            };
            assert!(send(&probe, messages));
            assert!(send(&probe, messages));
            let timeouts = inner.timeouts.borrow();
            assert_eq!(timeouts.len(), 2);
            assert!(timeouts[1] < timeouts[0]);
            assert!(timeouts[0] <= Duration::from_secs(10));
        }
    }

    #[test]
    fn successful_but_late_transport_response_is_rejected_for_both_protocols() {
        for messages in [false, true] {
            let inner = Rc::new(Probe {
                timeouts: RefCell::new(Vec::new()),
                late: true,
            });
            let probe = DeadlineTransport {
                inner: inner.clone(),
                deadline: Instant::now() + Duration::from_millis(500),
            };
            assert!(!send(&probe, messages));
            assert_eq!(inner.timeouts.borrow().len(), 1);
        }
    }

    struct LateEvidence {
        status: Option<u16>,
    }

    impl ProviderTransport for LateEvidence {
        fn send(
            &self,
            _: &str,
            _: Option<&str>,
            _: &str,
            timeout: Duration,
            _: bool,
        ) -> Result<TransportResponse, TransportError> {
            std::thread::sleep(timeout + Duration::from_millis(5));
            match self.status {
                Some(status) => Ok(TransportResponse::body(status, "")),
                None => Err(TransportError::UnsafeResponse),
            }
        }
    }

    impl AnthropicTransport for LateEvidence {
        fn send(
            &self,
            endpoint: &str,
            _: &AnthropicHeaders,
            body: &str,
            timeout: Duration,
            streaming: bool,
        ) -> Result<TransportResponse, AnthropicTransportError> {
            ProviderTransport::send(self, endpoint, None, body, timeout, streaming)
                .map_err(|_| AnthropicTransportError::UnsafeResponse)
        }
    }

    #[test]
    fn late_definitive_evidence_is_not_replaced_with_retryable_timeout() {
        for status in [None, Some(401), Some(403), Some(429), Some(503)] {
            let probe = DeadlineTransport {
                inner: Rc::new(LateEvidence { status }),
                deadline: Instant::now() + Duration::from_millis(100),
            };
            let result = ProviderTransport::send(
                &probe,
                "https://example.test",
                None,
                "{}",
                Duration::from_secs(30),
                false,
            );
            match status {
                Some(status) => assert_eq!(result.unwrap().status_code, status),
                None => assert!(matches!(result, Err(TransportError::UnsafeResponse))),
            }
            let probe = DeadlineTransport {
                inner: Rc::new(LateEvidence { status }),
                deadline: Instant::now() + Duration::from_millis(100),
            };
            let headers = AnthropicHeaders {
                api_key_ref: None,
                anthropic_version: "2023-06-01".into(),
            };
            let result = AnthropicTransport::send(
                &probe,
                "https://example.test",
                &headers,
                "{}",
                Duration::from_secs(30),
                false,
            );
            match status {
                Some(status) => assert_eq!(result.unwrap().status_code, status),
                None => assert!(matches!(
                    result,
                    Err(AnthropicTransportError::UnsafeResponse)
                )),
            }
        }
    }
}
