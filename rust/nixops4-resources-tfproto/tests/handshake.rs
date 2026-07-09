//! Round-trip integration test against the Go fixture.
//!
//! The fixture path is provided via the `NIXOPS4_TFPROTO_FIXTURE`
//! environment variable, set by the flake's test infrastructure.
//! When running under plain `cargo test` outside that setup, the test
//! is skipped rather than failing.

use nixops4_resources_tfproto::tf_provider_client::ProviderClient;

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
async fn empty_schema_fetch() {
    let Some(path) = fixture_path() else {
        eprintln!("skipping: NIXOPS4_TFPROTO_FIXTURE not set");
        return;
    };
    let client = ProviderClient::launch(&path)
        .await
        .expect("launch fixture");
    let schema = client
        .get_provider_schema()
        .await
        .expect("fetch empty schema");
    assert!(schema.resource_schemas.is_empty(), "no resource schemas");
    assert!(
        schema.data_source_schemas.is_empty(),
        "no data source schemas"
    );
    client.shutdown().await.expect("clean shutdown");
}
