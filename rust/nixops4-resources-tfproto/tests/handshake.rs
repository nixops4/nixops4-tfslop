//! Round-trip integration test against the Go fixture.
//!
//! The fixture path is provided via the `NIXOPS4_TFPROTO_FIXTURE`
//! environment variable, set by the flake's test infrastructure.
//! When running under plain `cargo test` outside that setup, the test
//! is skipped rather than failing.

use nixops4_resources_tfproto::{schema::ProviderSchema, tf_provider_client::ProviderClient};
use std::os::unix::process::ExitStatusExt;
use std::time::Duration;

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

// #37 R2: an RPC against a hung provider must not hang the adapter.
// Every RPC needs a bounded wait; blowing past it errors out so
// `nixops4 apply` can surface it rather than freeze.
#[tokio::test]
async fn plan_against_hanging_provider_times_out() {
    let Some(path) = fixture_path() else {
        eprintln!("skipping: NIXOPS4_TFPROTO_FIXTURE not set");
        return;
    };
    let client = ProviderClient::launch_with_rpc_timeout(&path, Duration::from_millis(500))
        .await
        .expect("launch fixture");

    let start = std::time::Instant::now();
    let result = client
        .plan_resource_change(
            "fx_hang_on_plan",
            b"null".to_vec(),
            b"{}".to_vec(),
            b"{}".to_vec(),
        )
        .await;
    let elapsed = start.elapsed();
    let _ = client.shutdown().await;

    let err = result.expect_err("Plan against a hanging provider must error");
    // Some slack — the timeout is 500ms, the polling loop and OS
    // scheduler add jitter, but the whole thing should still be
    // over well under the 2s budget.
    assert!(
        elapsed < Duration::from_secs(2),
        "timeout took too long: {elapsed:?}"
    );
    let chain = format!("{err:#}");
    assert!(
        chain.to_lowercase().contains("timeout") || chain.to_lowercase().contains("timed out"),
        "error should identify a timeout; got: {chain}"
    );
}

// #37 R1: dropping a ProviderClient without calling shutdown must
// not leak the child process. `shutdown()` is the graceful path;
// `Drop` is the safety net for the panic / forgotten-call path.
// A leaked child would linger until the test binary (or the
// eventual `nixops4` process) exited — a real orphan.
#[tokio::test]
async fn drop_without_shutdown_kills_child() {
    let Some(path) = fixture_path() else {
        eprintln!("skipping: NIXOPS4_TFPROTO_FIXTURE not set");
        return;
    };
    let client = ProviderClient::launch(&path).await.expect("launch fixture");
    let pid = client.pid().expect("child pid available after launch");
    drop(client);

    // Poll `kill(pid, None)` (an existence probe): Ok while the
    // process is alive, Err(ESRCH) when it's gone. A short deadline
    // — Drop should be effectively immediate.
    let nix_pid = nix::unistd::Pid::from_raw(pid as i32);
    let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
    loop {
        if nix::sys::signal::kill(nix_pid, None).is_err() {
            return;
        }
        if tokio::time::Instant::now() > deadline {
            panic!("child pid {pid} still alive 2s after ProviderClient drop");
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

// #37: shutdown() previously SIGKILL'd the child immediately, robbing
// any real provider of the chance to flush state, close connections,
// or run signal handlers. A clean shutdown must go through the
// tfplugin-native StopProvider RPC, then SIGTERM (go-plugin's
// documented graceful-shutdown signal); SIGKILL is only escalation
// of last resort.
//
// Observable proof: after clean shutdown against a well-behaved
// fixture, the child's ExitStatus must NOT be a SIGKILL. It may be
// exit(0) (if the provider installs a signal handler that exits
// cleanly) or termination by SIGTERM (go-plugin's default). Either
// is "graceful" — what we ruled out is "killed abruptly, no chance
// to clean up."
#[tokio::test]
async fn shutdown_does_not_sigkill_a_healthy_child() {
    let Some(path) = fixture_path() else {
        eprintln!("skipping: NIXOPS4_TFPROTO_FIXTURE not set");
        return;
    };
    let client = ProviderClient::launch(&path).await.expect("launch fixture");
    let status = client.shutdown().await.expect("shutdown");
    assert_ne!(
        status.signal(),
        Some(nix::sys::signal::Signal::SIGKILL as i32),
        "healthy child should not be SIGKILL'd during shutdown; got {status:?}"
    );
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
