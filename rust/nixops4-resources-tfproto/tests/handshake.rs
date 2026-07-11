//! Round-trip integration test against the Go fixture.
//!
//! The fixture path is provided via the `NIXOPS4_TFPROTO_FIXTURE`
//! environment variable, set by the flake's test infrastructure.
//! When running under plain `cargo test` outside that setup, the test
//! is skipped rather than failing.

use nixops4_resources_tfproto::{schema::ProviderSchema, tf_provider_client::ProviderClient};

fn fixture_path() -> Option<String> {
    std::env::var("NIXOPS4_TFPROTO_FIXTURE").ok()
}

#[tokio::test]
async fn spawn_handshake_disconnect() {
    let Some(path) = fixture_path() else {
        eprintln!("skipping: NIXOPS4_TFPROTO_FIXTURE not set");
        return;
    };
    let client = ProviderClient::launch(&path)
        .await
        .expect("launch and handshake against fixture");
    client.shutdown().await.expect("clean shutdown");
}

#[tokio::test]
async fn schema_string_attribute_roundtrip() {
    let Some(path) = fixture_path() else {
        eprintln!("skipping: NIXOPS4_TFPROTO_FIXTURE not set");
        return;
    };
    let client = ProviderClient::launch(&path).await.expect("launch fixture");
    let raw = client.get_provider_schema().await.expect("fetch schema");
    client.shutdown().await.expect("clean shutdown");

    let schema = ProviderSchema::from_wire(raw).expect("parse schema");

    assert!(schema.data_source_schemas.is_empty());
    let rs = schema
        .resource_schemas
        .get("fx_string")
        .expect("fx_string resource");
    assert_eq!(rs.block.attributes.len(), 1);
    let attr = rs.block.attributes.get("value").expect("value attribute");
    assert!(attr.required);
    assert!(!attr.optional);
    assert!(!attr.computed);
    // go-cty JSON forwards the type verbatim; a primitive `string` on the
    // wire is a JSON string, quotes included.
    assert_eq!(attr.cty_type, "\"string\"");
}

#[tokio::test]
async fn create_string_via_plan_apply() {
    let Some(path) = fixture_path() else {
        eprintln!("skipping: NIXOPS4_TFPROTO_FIXTURE not set");
        return;
    };
    let client = ProviderClient::launch(&path).await.expect("launch fixture");

    // The fixture takes no provider configuration; a null cty value
    // is the smallest legal one to send.
    client
        .configure_provider(b"null".to_vec())
        .await
        .expect("configure fixture");

    // A create is a Plan followed by an Apply. `prior_state` is a
    // cty null (no prior resource). `proposed_new_state` and `config`
    // both encode the user-requested configuration in go-cty JSON.
    let proposed = br#"{"value":"hello"}"#.to_vec();
    let plan = client
        .plan_resource_change(
            "fx_string",
            b"null".to_vec(),
            proposed.clone(),
            proposed.clone(),
        )
        .await
        .expect("plan create");
    let planned_state = plan.planned_state.expect("fixture returns planned state");

    let apply = client
        .apply_resource_change(
            "fx_string",
            b"null".to_vec(),
            planned_state,
            plan.planned_private,
            proposed,
        )
        .await
        .expect("apply create");
    let new_state = apply.new_state.expect("fixture returns new state");

    client.shutdown().await.expect("clean shutdown");

    // The fixture echoes proposed → planned → new. The JSON payload
    // survives the round trip byte-for-byte.
    assert_eq!(new_state.json, br#"{"value":"hello"}"#);
}

#[tokio::test]
async fn update_string_via_plan_apply() {
    let Some(path) = fixture_path() else {
        eprintln!("skipping: NIXOPS4_TFPROTO_FIXTURE not set");
        return;
    };
    let client = ProviderClient::launch(&path).await.expect("launch fixture");
    client
        .configure_provider(b"null".to_vec())
        .await
        .expect("configure fixture");

    // An update carries a real prior state and a new proposed state.
    // The fixture echoes proposed → planned → new; we assert the
    // *new* value round-trips, not the prior one, so a bug that
    // accidentally used prior_state would be caught.
    let prior = br#"{"value":"before"}"#.to_vec();
    let proposed = br#"{"value":"after"}"#.to_vec();
    let plan = client
        .plan_resource_change(
            "fx_string",
            prior.clone(),
            proposed.clone(),
            proposed.clone(),
        )
        .await
        .expect("plan update");
    let planned_state = plan.planned_state.expect("fixture returns planned state");

    let apply = client
        .apply_resource_change(
            "fx_string",
            prior,
            planned_state,
            plan.planned_private,
            proposed,
        )
        .await
        .expect("apply update");
    let new_state = apply.new_state.expect("fixture returns new state");
    client.shutdown().await.expect("clean shutdown");

    assert_eq!(new_state.json, br#"{"value":"after"}"#);
}

// #75: when the child process dies mid-RPC, tonic reports a generic
// transport error ("broken pipe"). The adapter should notice the
// exit and surface it. Two axes are probed — which RPC ("plan" or
// "apply") and which exit code (37 or 38) — so a hardcoded string
// in the adapter's output can't satisfy every combination.
#[tokio::test]
async fn mid_rpc_child_death_reports_exit_code() {
    let Some(path) = fixture_path() else {
        eprintln!("skipping: NIXOPS4_TFPROTO_FIXTURE not set");
        return;
    };

    for op in ["plan", "apply"] {
        for code in [37, 38] {
            let client = ProviderClient::launch(&path).await.expect("launch fixture");
            client
                .configure_provider(b"null".to_vec())
                .await
                .expect("configure fixture");

            let inputs =
                format!(r#"{{"crashing_operation":"{op}","exit_code":{code}}}"#).into_bytes();
            let err_chain = match op {
                "plan" => {
                    let result = client
                        .plan_resource_change("fx_die", b"null".to_vec(), inputs.clone(), inputs)
                        .await;
                    let _ = client.shutdown().await;
                    let err = result.expect_err("Plan against a dying provider must error");
                    format!("{err:#}")
                }
                "apply" => {
                    let plan = client
                        .plan_resource_change(
                            "fx_die",
                            b"null".to_vec(),
                            inputs.clone(),
                            inputs.clone(),
                        )
                        .await
                        .expect("plan must succeed for apply-crash case");
                    let planned_state = plan.planned_state.expect("fixture returns planned state");
                    let result = client
                        .apply_resource_change(
                            "fx_die",
                            b"null".to_vec(),
                            planned_state,
                            plan.planned_private,
                            inputs,
                        )
                        .await;
                    let _ = client.shutdown().await;
                    let err = result.expect_err("Apply against a dying provider must error");
                    format!("{err:#}")
                }
                _ => unreachable!(),
            };

            assert!(
                err_chain.contains(&code.to_string()),
                "code {code} missing from {op} error chain: {err_chain}"
            );
        }
    }
}

// #73: check_diagnostics is wrapped by every RPC call site with
// `.context("provider returned diagnostics")`. The wrapper string
// ends up above the actual message in what the user sees. The
// caller already names the RPC (via the outer `.context("calling
// PlanResourceChange")`), and the diagnostic itself is what
// happened — the middle wrapper is pure noise.
//
// This test drives an actual error diagnostic through
// `plan_resource_change` (by exploiting fx_write_only's guard) and
// asserts the resulting error chain does NOT include the wrapper.
#[tokio::test]
async fn plan_error_omits_returned_diagnostics_wrapper() {
    let Some(path) = fixture_path() else {
        eprintln!("skipping: NIXOPS4_TFPROTO_FIXTURE not set");
        return;
    };
    let client = ProviderClient::launch(&path).await.expect("launch fixture");
    client
        .configure_provider(b"null".to_vec())
        .await
        .expect("configure fixture");

    // fx_write_only's PlanResourceChange returns an error diagnostic
    // when prior_state contains `secret`. Call it directly at the
    // tfplugin level, bypassing the adapter's stripping.
    let prior = br#"{"value":"v0","secret":"leaked"}"#.to_vec();
    let proposed = br#"{"value":"v1"}"#.to_vec();
    let result = client
        .plan_resource_change("fx_write_only", prior, proposed.clone(), proposed)
        .await;
    client.shutdown().await.expect("clean shutdown");

    let err = result.expect_err("plan should have errored");
    let chain = format!("{err:#}");
    assert!(
        !chain.contains("provider returned diagnostics"),
        "wrapper phrase should not appear in the error chain: {chain}"
    );
    // Sanity: the actual provider message must still be visible.
    assert!(
        chain.contains("prior_state.secret leaked"),
        "provider's diagnostic summary is missing: {chain}"
    );
}

#[tokio::test]
async fn destroy_string_via_plan_apply() {
    let Some(path) = fixture_path() else {
        eprintln!("skipping: NIXOPS4_TFPROTO_FIXTURE not set");
        return;
    };
    let client = ProviderClient::launch(&path).await.expect("launch fixture");
    client
        .configure_provider(b"null".to_vec())
        .await
        .expect("configure fixture");

    // A destroy sends the prior state and a JSON null as the proposed
    // state. The provider echoes it back to new_state, signalling that
    // the resource is gone.
    let prior = br#"{"value":"doomed"}"#.to_vec();
    let null_state = b"null".to_vec();
    let plan = client
        .plan_resource_change(
            "fx_string",
            prior.clone(),
            null_state.clone(),
            null_state.clone(),
        )
        .await
        .expect("plan destroy");
    let planned_state = plan.planned_state.expect("fixture returns planned state");

    let apply = client
        .apply_resource_change(
            "fx_string",
            prior,
            planned_state,
            plan.planned_private,
            null_state,
        )
        .await
        .expect("apply destroy");
    let new_state = apply.new_state.expect("fixture returns new state");
    client.shutdown().await.expect("clean shutdown");

    assert_eq!(new_state.json, b"null");
}
