//! Integration test at the boundary between the nixops4 resource
//! protocol (JSON-line stdio) and the tfplugin gRPC protocol.
//!
//! Spawns the built `nixops4-resources-tfproto run` binary — the same
//! binary nixops4 would spawn — and drives it with a
//! `CreateResourceRequest`. Success proves the whole vertical is wired
//! up: adapter parses the request, launches the fixture, runs Plan +
//! Apply, and returns a response nixops4 accepts.

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
        provider_args: vec![
            "run".to_string(),
            "--provider-exe".to_string(),
            fixture,
        ],
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
        .update("fx_string", &new_inputs, &previous_inputs, &previous_outputs)
        .await
        .expect("update fx_string");

    assert!(
        outputs.is_empty(),
        "expected no output properties, got {:?}",
        outputs
    );

    client.close_wait().await.expect("close adapter");
}
