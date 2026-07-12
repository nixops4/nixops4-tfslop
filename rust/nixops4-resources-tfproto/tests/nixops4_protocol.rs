//! Integration test at the boundary between the nixops4 resource
//! protocol (JSON-line stdio) and the tfplugin gRPC protocol.
//!
//! Spawns the built `nixops4-resources-tfproto run` binary — the same
//! binary nixops4 would spawn — and drives it with a
//! `CreateResourceRequest`. Success proves the whole vertical is wired
//! up: adapter parses the request, launches the fixture, runs Plan +
//! Apply, and returns a response nixops4 accepts.

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

    let outputs = client
        .create("fx_string", &inputs, true)
        .await
        .expect("create fx_string");

    // fx_string declares no computed attributes; output is empty.
    assert!(
        outputs.is_empty(),
        "expected no output properties, got {:?}",
        outputs
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

    let outputs = client
        .update(
            "fx_string",
            &new_inputs,
            &previous_inputs,
            &previous_outputs,
        )
        .await
        .expect("update fx_string");

    assert!(
        outputs.is_empty(),
        "expected no output properties, got {:?}",
        outputs
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
        .expect("create fx_computed");

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
        .expect("create fx_computed_block_attr");

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

    // A successful call proves the adapter stripped `secret` from
    // prior_state before sending it to the provider. A failure here
    // means the guard regressed.
    let _ = client
        .update(
            "fx_write_only",
            &new_inputs,
            &previous_inputs,
            &previous_outputs,
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
    };

    client.destroy(resource).await.expect("destroy fx_string");

    client.close_wait().await.expect("close adapter");
}
