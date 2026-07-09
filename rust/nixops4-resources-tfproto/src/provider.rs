//! `nixops4_resource::ResourceProvider` implementation that drives a
//! tfplugin gRPC provider.

use anyhow::{Context, Result};
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
        let inputs_json = properties_to_cty_json(request.input_properties.0)?;
        let outputs = self
            .plan_and_apply(&type_name, b"null".to_vec(), inputs_json)
            .await?;
        Ok(v0::CreateResourceResponse {
            output_properties: outputs,
        })
    }

    async fn update(
        &self,
        request: v0::UpdateResourceRequest,
    ) -> Result<v0::UpdateResourceResponse> {
        let type_name = request.resource.type_.0.clone();
        let new_inputs_json = properties_to_cty_json(request.input_properties.0)?;
        // The tfplugin protocol wants the prior *full* state, not just
        // the user-controlled inputs. Reconstruct that by layering the
        // provider's previously-returned outputs on top of the previous
        // inputs. Same field name in both wins the later value, which
        // matches how the provider itself would have merged them.
        let mut prior_map: Map<String, Value> = request.resource.input_properties.0.into_iter().collect();
        if let Some(prev_outputs) = request.resource.output_properties {
            for (k, v) in prev_outputs.0 {
                prior_map.insert(k, v);
            }
        }
        let prior_state_json =
            serde_json::to_vec(&Value::Object(prior_map)).context("serialising prior state")?;
        let outputs = self
            .plan_and_apply(&type_name, prior_state_json, new_inputs_json)
            .await?;
        Ok(v0::UpdateResourceResponse {
            output_properties: outputs,
        })
    }
}

impl TfPluginProvider {
    /// Run the Configure → Plan → Apply sequence for one resource
    /// change, spawning and reaping a fresh tfplugin provider process
    /// around it.
    async fn plan_and_apply(
        &self,
        type_name: &str,
        prior_state_json: Vec<u8>,
        inputs_json: Vec<u8>,
    ) -> Result<v0::OutputProperties> {
        let client = ProviderClient::launch(&self.provider_exe).await?;
        let result =
            plan_and_apply_inner(&client, type_name, prior_state_json, inputs_json).await;
        client.shutdown().await?;
        result
    }
}

async fn plan_and_apply_inner(
    client: &ProviderClient,
    type_name: &str,
    prior_state_json: Vec<u8>,
    inputs_json: Vec<u8>,
) -> Result<v0::OutputProperties> {
    // The fixture takes no configuration. Real providers configured
    // through the resource `tf-provider-*` prefix mechanism land later
    // in the series.
    client.configure_provider(b"null".to_vec()).await?;

    let plan = client
        .plan_resource_change(
            type_name,
            prior_state_json.clone(),
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
            prior_state_json,
            planned_state,
            plan.planned_private,
            inputs_json,
        )
        .await?;
    let _new_state = apply.new_state.context("provider returned no new_state")?;

    // No computed attributes across the surface we test today. When we
    // add them, project `new_state` onto only the schema's `computed`
    // fields — the protocol explicitly discourages echoing inputs.
    Ok(v0::OutputProperties(Map::new()))
}

fn properties_to_cty_json(props: Map<String, Value>) -> Result<Vec<u8>> {
    serde_json::to_vec(&Value::Object(props)).context("serialising resource properties")
}
