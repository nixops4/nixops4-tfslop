//! `nixops4_resource::ResourceProvider` implementation that drives a
//! tfplugin gRPC provider.

use anyhow::{bail, Context, Result};
use nixops4_resource::framework::ResourceProvider;
use nixops4_resource::schema::v0;
use serde_json::{Map, Value};

use crate::tf_provider_client::ProviderClient;

pub struct TfPluginProvider {
    /// Path to the tfplugin provider binary this adapter drives.
    provider_exe: String,
}

impl TfPluginProvider {
    pub fn new(provider_exe: String) -> Self {
        Self { provider_exe }
    }
}

impl ResourceProvider for TfPluginProvider {
    async fn create(
        &self,
        request: v0::CreateResourceRequest,
    ) -> Result<v0::CreateResourceResponse> {
        let type_name = request.type_.0.clone();
        // The provider expects the config as go-cty JSON; the resource
        // schema is a top-level object so a serde_json map serializes to
        // the right shape.
        let inputs_json = serde_json::to_vec(&Value::Object(
            request
                .input_properties
                .0
                .into_iter()
                .collect::<Map<_, _>>(),
        ))
        .context("serialising input properties")?;

        let client = ProviderClient::launch(&self.provider_exe).await?;
        let result = create_inner(&client, &type_name, inputs_json).await;
        client.shutdown().await?;
        result
    }

    async fn update(
        &self,
        _request: v0::UpdateResourceRequest,
    ) -> Result<v0::UpdateResourceResponse> {
        bail!("nixops4-resources-tfproto: update is not implemented yet")
    }
}

async fn create_inner(
    client: &ProviderClient,
    type_name: &str,
    inputs_json: Vec<u8>,
) -> Result<v0::CreateResourceResponse> {
    // The fixture takes no configuration; real providers configured
    // through the resource `tf-provider-*` prefix mechanism will land
    // later in the series.
    client.configure_provider(b"null".to_vec()).await?;

    let plan = client
        .plan_resource_change(
            type_name,
            b"null".to_vec(),
            inputs_json.clone(),
            inputs_json.clone(),
        )
        .await?;
    let planned_state = plan
        .planned_state
        .context("provider returned no planned_state")?;

    let apply = client
        .apply_resource_change(
            type_name,
            b"null".to_vec(),
            planned_state,
            plan.planned_private,
            inputs_json,
        )
        .await?;
    let _new_state = apply.new_state.context("provider returned no new_state")?;

    // No computed attributes for our current test surface; output is
    // empty. When we add computed attributes, filter `new_state` to
    // only those the schema marks `computed`.
    Ok(v0::CreateResourceResponse {
        output_properties: v0::OutputProperties(Map::new()),
    })
}
