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

    // The fixture declares exactly one resource with one required string.
    assert!(schema.data_source_schemas.is_empty());
    assert_eq!(schema.resource_schemas.len(), 1);
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
