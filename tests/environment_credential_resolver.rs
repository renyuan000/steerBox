use steerbox::http_transport::{
    CredentialError, CredentialResolver, EnvironmentCredentialResolver,
};

fn unique_env_name(suffix: &str) -> String {
    format!("STEERBOX_TEST_CREDENTIAL_{}_{}", std::process::id(), suffix)
}

#[test]
fn environment_resolver_reads_only_env_references() {
    let name = unique_env_name("present");
    // SAFETY: this test owns its process-unique variable and removes it below.
    unsafe { std::env::set_var(&name, "fixture-secret-value") };

    let resolved = EnvironmentCredentialResolver.resolve(&format!("env:{name}"));

    // SAFETY: this test owns its process-unique variable.
    unsafe { std::env::remove_var(&name) };
    assert!(resolved.is_ok());
    assert_eq!(
        format!("{:?}", resolved.unwrap()),
        "ResolvedCredential([REDACTED])"
    );
}

#[test]
fn environment_resolver_reports_missing_values_without_exposing_reference_or_value() {
    let name = unique_env_name("missing");
    // SAFETY: this test owns its process-unique variable.
    unsafe { std::env::remove_var(&name) };

    let error = EnvironmentCredentialResolver
        .resolve(&format!("env:{name}"))
        .expect_err("unset environment variable must fail closed");

    assert_eq!(error, CredentialError::Missing);
    assert!(!error.to_string().contains(&name));
}

#[test]
fn environment_resolver_rejects_non_env_and_invalid_variable_references() {
    for auth_ref in [
        "",
        "secret-ref:fixture",
        "env:",
        "env:with-dash",
        "env:1STARTS_WITH_DIGIT",
        "env:HAS SPACE",
        "env:HAS=EQUALS",
        "env:NAME:EXTRA",
    ] {
        assert!(
            matches!(
                EnvironmentCredentialResolver.resolve(auth_ref),
                Err(CredentialError::Rejected)
            ),
            "invalid auth_ref must be rejected: {auth_ref:?}"
        );
    }
}
