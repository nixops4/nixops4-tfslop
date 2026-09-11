//! Integration test at the boundary between the nixops4 resource
//! protocol (JSON-line stdio) and the tfplugin gRPC protocol.
//!
//! Spawns the built `nixops4-resources-tfproto run` binary — the same
//! binary nixops4 would spawn — and drives it with a
//! `CreateResourceRequest`. Success proves the whole vertical is wired
//! up: adapter parses the request, launches the fixture, runs Plan +
//! Apply, and returns a response nixops4 accepts.

use base64::Engine as _;
use nixops4_resource::schema::v0;
use nixops4_resource_runner::{ResourceProviderClient, ResourceProviderConfig};
use serde_json::{Map, Value};

const ADAPTER: &str = env!("CARGO_BIN_EXE_nixops4-resources-tfproto");

fn fixture_path() -> Option<String> {
    std::env::var("NIXOPS4_TFPROTO_FIXTURE").ok()
}

#[tokio::test]
async fn create_fx_string_via_nixops4_protocol() {
    let Some(fixture) = fixture_path() else {
        eprintln!("skipping: NIXOPS4_TFPROTO_FIXTURE not set");
        return;
    };

    let mut client = ResourceProviderClient::new(ResourceProviderConfig {
        provider_executable: ADAPTER.to_string(),
        provider_args: vec!["run".to_string(), "--provider-exe".to_string(), fixture],
    })
    .await
    .expect("spawn adapter");

    let mut inputs = Map::new();
    inputs.insert("value".to_string(), Value::String("hello".to_string()));

    let outcome = client
        .create("fx_string", &inputs, true)
        .await
        .expect("create fx_string");

    // fx_string declares no computed attributes; output is empty.
    assert!(
        outcome.output_properties.is_empty(),
        "expected no output properties, got {:?}",
        outcome.output_properties
    );

    client.close_wait().await.expect("close adapter");
}

#[tokio::test]
async fn update_fx_string_via_nixops4_protocol() {
    let Some(fixture) = fixture_path() else {
        eprintln!("skipping: NIXOPS4_TFPROTO_FIXTURE not set");
        return;
    };

    let mut client = ResourceProviderClient::new(ResourceProviderConfig {
        provider_executable: ADAPTER.to_string(),
        provider_args: vec!["run".to_string(), "--provider-exe".to_string(), fixture],
    })
    .await
    .expect("spawn adapter");

    // Previous state as nixops4 would have persisted it after a create
    // with value = "before". No computed outputs (fx_string has none).
    let mut previous_inputs = Map::new();
    previous_inputs.insert("value".to_string(), Value::String("before".to_string()));
    let previous_outputs = Map::new();

    let mut new_inputs = Map::new();
    new_inputs.insert("value".to_string(), Value::String("after".to_string()));

    let outcome = client
        .update(
            "fx_string",
            &new_inputs,
            &previous_inputs,
            &previous_outputs,
            None,
        )
        .await
        .expect("update fx_string");

    assert!(
        outcome.output_properties.is_empty(),
        "expected no output properties, got {:?}",
        outcome.output_properties
    );

    client.close_wait().await.expect("close adapter");
}

#[tokio::test]
async fn create_fx_computed_projects_id_output() {
    let Some(fixture) = fixture_path() else {
        eprintln!("skipping: NIXOPS4_TFPROTO_FIXTURE not set");
        return;
    };

    let mut client = ResourceProviderClient::new(ResourceProviderConfig {
        provider_executable: ADAPTER.to_string(),
        provider_args: vec!["run".to_string(), "--provider-exe".to_string(), fixture],
    })
    .await
    .expect("spawn adapter");

    let mut inputs = Map::new();
    inputs.insert("value".to_string(), Value::String("nixops4".to_string()));

    let outputs = client
        .create("fx_computed", &inputs, true)
        .await
        .expect("create fx_computed")
        .output_properties;

    // The fixture populates `id = "computed-" + value` during Plan.
    // The user-provided `value` must not leak into outputs.
    assert_eq!(
        outputs.get("id"),
        Some(&Value::String("computed-nixops4".to_string())),
        "id output missing or wrong: {:?}",
        outputs
    );
    assert!(
        !outputs.contains_key("value"),
        "value must not appear in outputs (it is user input, not computed): {:?}",
        outputs
    );

    client.close_wait().await.expect("close adapter");
}

#[tokio::test]
async fn create_fx_computed_block_projects_nested_token() {
    let Some(fixture) = fixture_path() else {
        eprintln!("skipping: NIXOPS4_TFPROTO_FIXTURE not set");
        return;
    };

    let mut client = ResourceProviderClient::new(ResourceProviderConfig {
        provider_executable: ADAPTER.to_string(),
        provider_args: vec!["run".to_string(), "--provider-exe".to_string(), fixture],
    })
    .await
    .expect("spawn adapter");

    // Populate the nested block; the fixture computes a token from the
    // endpoint.
    let mut settings = Map::new();
    settings.insert(
        "endpoint".to_string(),
        Value::String("https://example.invalid".to_string()),
    );
    let mut inputs = Map::new();
    inputs.insert("settings".to_string(), Value::Object(settings));

    let outputs = client
        .create("fx_computed_block_attr", &inputs, true)
        .await
        .expect("create fx_computed_block_attr")
        .output_properties;

    // Outputs preserve block structure. `endpoint` is a user input
    // and must not appear in outputs; `token` is computed and must.
    let out_settings = outputs
        .get("settings")
        .and_then(|v| v.as_object())
        .expect("settings block projected into outputs");
    assert_eq!(
        out_settings.get("token"),
        Some(&Value::String(
            "token-for-https://example.invalid".to_string()
        )),
        "token projection wrong: {:?}",
        out_settings
    );
    assert!(
        !out_settings.contains_key("endpoint"),
        "endpoint (user input) must not appear in outputs: {:?}",
        out_settings
    );

    client.close_wait().await.expect("close adapter");
}

#[tokio::test]
async fn update_fx_write_only_strips_secret_from_prior_state() {
    let Some(fixture) = fixture_path() else {
        eprintln!("skipping: NIXOPS4_TFPROTO_FIXTURE not set");
        return;
    };

    let mut client = ResourceProviderClient::new(ResourceProviderConfig {
        provider_executable: ADAPTER.to_string(),
        provider_args: vec!["run".to_string(), "--provider-exe".to_string(), fixture],
    })
    .await
    .expect("spawn adapter");

    // Simulate a resource that was previously created with both `value`
    // and `secret`. If nixops4 accidentally persisted the secret and
    // the adapter fed it back in prior_state, the fixture Plan errors.
    let mut previous_inputs = Map::new();
    previous_inputs.insert("value".to_string(), Value::String("v0".to_string()));
    previous_inputs.insert(
        "secret".to_string(),
        Value::String("previous-secret".to_string()),
    );
    let previous_outputs = Map::new();

    let mut new_inputs = Map::new();
    new_inputs.insert("value".to_string(), Value::String("v1".to_string()));
    new_inputs.insert(
        "secret".to_string(),
        Value::String("new-secret".to_string()),
    );

    // A successful call proves the adapter nulled `secret` in
    // prior_state before sending it to the provider. A failure here
    // means the guard regressed.
    let _ = client
        .update(
            "fx_write_only",
            &new_inputs,
            &previous_inputs,
            &previous_outputs,
            None,
        )
        .await
        .expect("update fx_write_only");

    client.close_wait().await.expect("close adapter");
}

// #43: the adapter must call ValidateResourceConfig before Plan so
// config errors surface at the earliest layer. A provider whose
// Validate rejects but whose Plan wouldn't would otherwise let the
// error go unheard — Plan produces some default and the user's
// broken config gets applied.
#[tokio::test]
async fn create_calls_validate_resource_config_before_plan() {
    let Some(fixture) = fixture_path() else {
        eprintln!("skipping: NIXOPS4_TFPROTO_FIXTURE not set");
        return;
    };

    let mut client = ResourceProviderClient::new(ResourceProviderConfig {
        provider_executable: ADAPTER.to_string(),
        provider_args: vec!["run".to_string(), "--provider-exe".to_string(), fixture],
    })
    .await
    .expect("spawn adapter");

    let inputs = Map::new();
    let result = client
        .create("fx_validate_always_rejects", &inputs, true)
        .await;

    client.close_wait().await.expect("close adapter");

    let err = result.expect_err("create must surface ValidateResourceConfig rejection");
    let msg = format!("{err:#}");
    assert!(
        msg.contains("validate rejected the resource config"),
        "adapter must surface the Validate diagnostic; got: {msg}"
    );
}

#[tokio::test]
async fn destroy_fx_string_via_nixops4_protocol() {
    let Some(fixture) = fixture_path() else {
        eprintln!("skipping: NIXOPS4_TFPROTO_FIXTURE not set");
        return;
    };

    let mut client = ResourceProviderClient::new(ResourceProviderConfig {
        provider_executable: ADAPTER.to_string(),
        provider_args: vec!["run".to_string(), "--provider-exe".to_string(), fixture],
    })
    .await
    .expect("spawn adapter");

    let mut prev_inputs = Map::new();
    prev_inputs.insert("value".to_string(), Value::String("bye".to_string()));

    let resource = v0::ExtantResource {
        type_: v0::ResourceType("fx_string".to_string()),
        input_properties: v0::InputProperties(prev_inputs),
        output_properties: Some(v0::OutputProperties(Map::new())),
        private_state: None,
    };

    client.destroy(resource).await.expect("destroy fx_string");

    client.close_wait().await.expect("close adapter");
}

// #54: env vars must inherit through nixops4 → adapter → provider.
// Real providers need this for credentials (AWS_*, KUBECONFIG, ...).
// The chain is subprocess-only and neither hop calls `env_clear`;
// this test locks that in so a future hermetic-spawn change can't
// silently break real-provider usage.
#[tokio::test]
async fn env_vars_inherit_through_adapter_to_provider() {
    let Some(fixture) = fixture_path() else {
        eprintln!("skipping: NIXOPS4_TFPROTO_FIXTURE not set");
        return;
    };

    let sentinel_name = "NIXOPS4_TFPROTO_ENV_INHERIT_SENTINEL";
    let sentinel_value = "carrot-42";
    // SAFETY: single-threaded tokio test, no other threads read this
    // var. `std::env::set_var` is unsafe on multi-threaded processes
    // because it races with libc `getenv` in other threads.
    unsafe { std::env::set_var(sentinel_name, sentinel_value) };

    let mut client = ResourceProviderClient::new(ResourceProviderConfig {
        provider_executable: ADAPTER.to_string(),
        provider_args: vec!["run".to_string(), "--provider-exe".to_string(), fixture],
    })
    .await
    .expect("spawn adapter");

    let mut inputs = Map::new();
    inputs.insert(
        "var_name".to_string(),
        Value::String(sentinel_name.to_string()),
    );

    let outputs = client
        .create("fx_env_echo", &inputs, true)
        .await
        .expect("create fx_env_echo")
        .output_properties;

    assert_eq!(
        outputs.get("value"),
        Some(&Value::String(sentinel_value.to_string())),
        "provider must see the sentinel env var set by the test; got {:?}",
        outputs
    );

    client.close_wait().await.expect("close adapter");
}

// #40: `private_state` end-to-end. Create must surface a non-None
// private_state derived from Apply.private. Update must feed that
// value back so the fixture's Plan sees the previously-emitted bytes
// as prior_private, and the freshly returned private_state must
// reflect the new token.
//
// Encoding is opaque to nixops4 — the adapter picks it — so this test
// asserts round-trip semantics without pinning the specific JSON
// shape: whatever create returned must survive back to the fixture
// unchanged and the fixture's prior-private assertion must hold.
#[tokio::test]
async fn fx_private_round_trips_between_create_and_update() {
    let Some(fixture) = fixture_path() else {
        eprintln!("skipping: NIXOPS4_TFPROTO_FIXTURE not set");
        return;
    };

    let mut client = ResourceProviderClient::new(ResourceProviderConfig {
        provider_executable: ADAPTER.to_string(),
        provider_args: vec!["run".to_string(), "--provider-exe".to_string(), fixture],
    })
    .await
    .expect("spawn adapter");

    let mut create_inputs = Map::new();
    create_inputs.insert("token".to_string(), Value::String("alpha".to_string()));

    let create_outcome = client
        .create("fx_private", &create_inputs, true)
        .await
        .expect("create fx_private");

    let private_after_create = create_outcome
        .private_state
        .clone()
        .expect("create must surface Apply.private as private_state");

    // Prove the create outcome is what the fixture handed us, not some
    // fabricated placeholder: base64-decode and check the payload.
    // (The fixture writes `private-for-<token>`.)
    let created_b64 = private_after_create
        .as_str()
        .expect("private_state must round-trip as a JSON string");
    let created_bytes = base64::engine::general_purpose::STANDARD
        .decode(created_b64)
        .expect("private_state must decode as base64");
    assert_eq!(
        created_bytes, b"private-for-alpha",
        "adapter must faithfully surface the bytes the provider chose"
    );

    // Update: pass `expected_prior_b64 = created_b64`. The fixture's
    // Plan checks `base64(prior_private) == expected_prior_b64` and
    // errors otherwise — so a passing call is proof the adapter
    // threaded private_state → prior_private.
    let mut update_inputs = Map::new();
    update_inputs.insert("token".to_string(), Value::String("beta".to_string()));
    update_inputs.insert(
        "expected_prior_b64".to_string(),
        Value::String(created_b64.to_string()),
    );

    let update_outcome = client
        .update(
            "fx_private",
            &update_inputs,
            &create_inputs,
            &Map::new(),
            Some(private_after_create),
        )
        .await
        .expect("update fx_private (adapter must feed prior private_state back to Plan)");

    let updated_b64 = update_outcome
        .private_state
        .as_ref()
        .and_then(|v| v.as_str())
        .expect("update must surface a fresh private_state");
    let updated_bytes = base64::engine::general_purpose::STANDARD
        .decode(updated_b64)
        .expect("updated private_state must decode as base64");
    assert_eq!(
        updated_bytes, b"private-for-beta",
        "update must reflect the rotated private state from the provider"
    );

    client.close_wait().await.expect("close adapter");
}

// Destroy must also thread private_state back into prior_private, or
// providers that clean up private-side bookkeeping on destroy will
// silently misbehave. Uses the same fx_private guard, but on the
// destroy path (proposed_new_state = null).
#[tokio::test]
async fn destroy_fx_private_receives_prior_private_state() {
    let Some(fixture) = fixture_path() else {
        eprintln!("skipping: NIXOPS4_TFPROTO_FIXTURE not set");
        return;
    };

    let mut client = ResourceProviderClient::new(ResourceProviderConfig {
        provider_executable: ADAPTER.to_string(),
        provider_args: vec!["run".to_string(), "--provider-exe".to_string(), fixture],
    })
    .await
    .expect("spawn adapter");

    // Pretend a create previously stored some private bytes. Since
    // fx_private echoes `private-for-<token>`, we compute the same
    // base64 the create path would produce.
    let stored = base64::engine::general_purpose::STANDARD.encode(b"private-for-gamma");
    let mut prev_inputs = Map::new();
    prev_inputs.insert("token".to_string(), Value::String("gamma".to_string()));
    // Flag on prior_state that tells the fixture to enforce the
    // destroy-time non-empty-prior_private guard. Without it, destroy
    // wouldn't have any way to observe a dropped private_state (the
    // proposed_new_state is null and the inputs are gone).
    prev_inputs.insert("is_destroy_probe".to_string(), Value::Bool(true));

    let resource = v0::ExtantResource {
        type_: v0::ResourceType("fx_private".to_string()),
        input_properties: v0::InputProperties(prev_inputs),
        output_properties: Some(v0::OutputProperties(Map::new())),
        private_state: Some(v0::PrivateState(Value::String(stored))),
    };

    client
        .destroy(resource)
        .await
        .expect("destroy fx_private must round-trip stored private_state without error");

    client.close_wait().await.expect("close adapter");
}
