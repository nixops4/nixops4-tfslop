//! `nixops4_resource::ResourceProvider` implementation that drives a
//! tfplugin gRPC provider.

use anyhow::{Context, Result};
use nixops4_resource::framework::ResourceProvider;
use nixops4_resource::schema::v0;
use serde_json::{Map, Value};

use crate::deprecation::collect_deprecated_hits;
use crate::outputs::project_new_state;
use crate::prior_state::build_prior_state_json;
use crate::private_state;
use crate::schema::ProviderSchema;
use crate::tf_provider_client::ProviderClient;

/// Prior state supplied by the caller. `None` marks a create; the
/// schema-aware reconstruction happens after the schema is fetched
/// inside `plan_and_apply_inner`.
struct PriorState {
    inputs: Map<String, Value>,
    outputs: Option<Map<String, Value>>,
}

/// Whole outcome of one Plan+Apply run: outputs projected onto the
/// declared schema, plus the opaque `private` bytes the provider chose
/// to persist for the next cycle.
struct PlanApplyOutcome {
    outputs: v0::OutputProperties,
    apply_private: Vec<u8>,
}

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
        let outcome = self
            .plan_and_apply(&type_name, None, Vec::new(), inputs_json)
            .await?;
        Ok(v0::CreateResourceResponse {
            output_properties: outcome.outputs,
            private_state: private_state::encode(outcome.apply_private).map(v0::PrivateState),
        })
    }

    async fn update(
        &self,
        request: v0::UpdateResourceRequest,
    ) -> Result<v0::UpdateResourceResponse> {
        let type_name = request.resource.type_.0.clone();
        let new_inputs_json = properties_to_cty_json(request.input_properties.0)?;
        let prior_private = private_state::decode(request.resource.private_state.map(|p| p.0))?;
        let prior = PriorState {
            inputs: request.resource.input_properties.0,
            outputs: request.resource.output_properties.map(|op| op.0),
        };
        let outcome = self
            .plan_and_apply(&type_name, Some(prior), prior_private, new_inputs_json)
            .await?;
        Ok(v0::UpdateResourceResponse {
            output_properties: outcome.outputs,
            private_state: private_state::encode(outcome.apply_private).map(v0::PrivateState),
        })
    }

    async fn destroy(
        &self,
        request: v0::DestroyResourceRequest,
    ) -> Result<v0::DestroyResourceResponse> {
        let type_name = request.resource.type_.0.clone();
        let prior_private = private_state::decode(request.resource.private_state.map(|p| p.0))?;
        let prior = PriorState {
            inputs: request.resource.input_properties.0,
            outputs: request.resource.output_properties.map(|op| op.0),
        };
        // A tfplugin destroy is Plan + Apply with `proposed_new_state`
        // = `planned_state` = `config` = cty null. The provider echoes
        // that null through to `new_state`, and any output properties
        // the provider might have returned are discarded — nixops4
        // will drop the resource from state. The `private` bytes are
        // likewise discarded: the resource is gone.
        let _ = self
            .plan_and_apply(&type_name, Some(prior), prior_private, b"null".to_vec())
            .await?;
        Ok(v0::DestroyResourceResponse {})
    }
}

impl TfPluginProvider {
    /// Run the Configure → Plan → Apply sequence for one resource
    /// change, spawning and reaping a fresh tfplugin provider process
    /// around it.
    async fn plan_and_apply(
        &self,
        type_name: &str,
        prior: Option<PriorState>,
        prior_private: Vec<u8>,
        inputs_json: Vec<u8>,
    ) -> Result<PlanApplyOutcome> {
        let client = ProviderClient::launch(&self.provider_exe).await?;
        let result =
            plan_and_apply_inner(&client, type_name, prior, prior_private, inputs_json).await;
        client.shutdown().await?;
        result
    }
}

async fn plan_and_apply_inner(
    client: &ProviderClient,
    type_name: &str,
    prior: Option<PriorState>,
    prior_private: Vec<u8>,
    inputs_json: Vec<u8>,
) -> Result<PlanApplyOutcome> {
    // Schema is fetched once per adapter invocation. Both the
    // projection of `new_state` onto declared computed outputs and
    // the stripping of write_only attributes from `prior_state` need
    // it — we cannot reliably identify either without the schema.
    let raw_schema = client.get_provider_schema().await?;
    let schema = ProviderSchema::from_proto(raw_schema)?;

    let prior_state_json = match prior {
        None => b"null".to_vec(),
        Some(p) => build_prior_state_json(&schema, type_name, p.inputs, p.outputs)?,
    };

    // Deprecation warnings are cross-referenced against inputs so we
    // only nag about fields the user actually set. Skip the destroy
    // path where inputs_json is the JSON null we send to tell the
    // provider to tear the resource down.
    if let Ok(Value::Object(inputs_map)) = serde_json::from_slice::<Value>(&inputs_json) {
        for msg in collect_deprecated_hits(&schema, type_name, &inputs_map) {
            eprintln!("{}", msg);
        }
    }

    // The fixture takes no configuration. Real providers configured
    // through the resource `tf-provider-*` prefix mechanism land later
    // in the series.
    client.configure_provider(b"null".to_vec()).await?;

    // Validate the resource config before Plan. Surfaces schema-shape
    // and provider-invariant errors at the earliest layer, where the
    // provider's message is most specific to the user's config — Plan
    // would either error later with a less-specific message, or (worse)
    // silently substitute defaults for the bad input.
    client
        .validate_resource_config(type_name, inputs_json.clone())
        .await?;

    let plan = client
        .plan_resource_change(
            type_name,
            prior_state_json.clone(),
            inputs_json.clone(),
            inputs_json.clone(),
            prior_private,
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
    let new_state = apply.new_state.context("provider returned no new_state")?;

    let outputs = project_new_state(&schema, type_name, &new_state.json)?;
    Ok(PlanApplyOutcome {
        outputs: v0::OutputProperties(outputs),
        apply_private: apply.private,
    })
}

fn properties_to_cty_json(props: Map<String, Value>) -> Result<Vec<u8>> {
    serde_json::to_vec(&Value::Object(props)).context("serialising resource properties")
}
