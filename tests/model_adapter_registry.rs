use steerbox::model_adapter::{
    LocalFixtureModelAdapter, ModelAdapterRegistry, ModelAdapterRegistryError, ModelDecision,
    ModelRequest,
};

fn request(model_id: &str) -> ModelRequest {
    ModelRequest {
        model_id: model_id.into(),
        goal_id: "goal-fixture".into(),
        loop_run_id: "loop-fixture".into(),
        step_id: "step-fixture".into(),
        objective: "Verify a local registry lookup".into(),
        observation: "Deterministic fixture only".into(),
    }
}

#[test]
fn blank_model_ids_are_rejected_without_registering_an_adapter() {
    let mut registry = ModelAdapterRegistry::default();

    for model_id in ["", " ", "\t\n"] {
        let result = registry.register(
            model_id,
            Box::new(LocalFixtureModelAdapter::new(
                ModelDecision::CompleteLoop,
                "unregistered fixture",
            )),
        );

        assert_eq!(result, Err(ModelAdapterRegistryError::EmptyModelId));
        assert!(registry.get(model_id).is_none());
    }
}

#[test]
fn duplicate_registration_does_not_replace_the_original_adapter() {
    let mut registry = ModelAdapterRegistry::default();
    registry
        .register(
            "fixture-model",
            Box::new(LocalFixtureModelAdapter::new(
                ModelDecision::ContinueObservation,
                "original fixture",
            )),
        )
        .expect("original adapter registers");

    let result = registry.register(
        "fixture-model",
        Box::new(LocalFixtureModelAdapter::new(
            ModelDecision::RequestHuman,
            "replacement fixture",
        )),
    );
    assert_eq!(
        result,
        Err(ModelAdapterRegistryError::DuplicateModel(
            "fixture-model".into()
        ))
    );

    let response = registry
        .get("fixture-model")
        .expect("original adapter remains registered")
        .complete(request("fixture-model"))
        .expect("valid local fixture request succeeds");
    assert_eq!(response.decision, ModelDecision::ContinueObservation);
    assert_eq!(response.summary, "original fixture");
}

#[test]
fn distinct_model_ids_resolve_to_their_own_adapters() {
    let mut registry = ModelAdapterRegistry::default();
    registry
        .register(
            "fixture-observe",
            Box::new(LocalFixtureModelAdapter::new(
                ModelDecision::ContinueObservation,
                "observation fixture",
            )),
        )
        .expect("observation adapter registers");
    registry
        .register(
            "fixture-complete",
            Box::new(LocalFixtureModelAdapter::new(
                ModelDecision::CompleteLoop,
                "completion fixture",
            )),
        )
        .expect("completion adapter registers");

    for (model_id, expected_decision, expected_summary) in [
        (
            "fixture-observe",
            ModelDecision::ContinueObservation,
            "observation fixture",
        ),
        (
            "fixture-complete",
            ModelDecision::CompleteLoop,
            "completion fixture",
        ),
    ] {
        let response = registry
            .get(model_id)
            .expect("specific adapter exists")
            .complete(request(model_id))
            .expect("valid local fixture request succeeds");
        assert_eq!(response.decision, expected_decision);
        assert_eq!(response.summary, expected_summary);
    }
}

#[test]
fn missing_model_lookup_returns_none_in_empty_and_populated_registries() {
    let mut registry = ModelAdapterRegistry::default();
    assert!(registry.get("fixture-missing").is_none());

    registry
        .register(
            "fixture-known",
            Box::new(LocalFixtureModelAdapter::new(
                ModelDecision::CompleteLoop,
                "known fixture",
            )),
        )
        .expect("known adapter registers");

    assert!(registry.get("fixture-missing").is_none());
    assert!(registry.get("fixture-known").is_some());
}
