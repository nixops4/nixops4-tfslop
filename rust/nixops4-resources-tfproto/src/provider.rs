//! `nixops4_resource::ResourceProvider` implementation that drives a
//! tfplugin gRPC provider.

use anyhow::{Context, Result};
use nixops4_resource::framework::{ResourceProvider, UpdateOutcome};
use nixops4_resource::schema::v0;
use serde_json::{Map, Value};

use crate::deprecation::collect_deprecated_hits;
use crate::outputs::project_new_state;
use crate::prior_state::build_prior_state_json;
use crate::private_state;
use crate::schema::ProviderSchema;
use crate::sensitive_markers::wrap_sensitive_outputs;
use crate::tf_provider_client::ProviderClient;

/// Prior state supplied by the caller. `None` marks a create; the
/// schema-aware reconstruction happens after the schema is fetched
/// inside `plan_and_apply_inner`.
struct PriorState {
    inputs: Map<String, Value>,
    outputs: Option<Map<String, Value>>,
    /// Complete state from the last Apply, when the adapter kept it.
    state: Option<Value>,
}

/// What a Plan came to.
///
/// A plan that names `requires_replace` paths says the change cannot be
/// applied to the resource that exists. Nothing is applied then: the answer
/// goes back to nixops4, which destroys the resource and creates it again.
enum PlanOutcome {
    Applied(PlanApplyOutcome),
    RequiresReplace { reasons: Vec<String> },
}

/// What the caller wants done when a Plan names `requires_replace`.
#[derive(Clone, Copy, PartialEq, Eq)]
enum OnRequiresReplace {
    /// Apply anyway. What a create does: the provider compared the config
    /// against a null prior, so terraform-plugin-sdk v2 names every ForceNew
    /// attribute, and the resource is being created rather than replaced.
    Ignore,
    /// Hand the paths back, so the caller can answer nixops4 with a
    /// replacement rather than applying a change the provider refused.
    Report,
}

/// Whole outcome of one Plan+Apply run: outputs projected onto the
/// declared schema, plus the opaque `private` bytes the provider chose
/// to persist for the next cycle.
struct PlanApplyOutcome {
    outputs: v0::OutputProperties,
    apply_private: Vec<u8>,
    /// The provider's complete `new_state`, kept for the next prior_state.
    new_state: Value,
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
        let (inputs, provider_config) = crate::provider_config::split(request.input_properties.0)?;
        let inputs_json = properties_to_cty_json(inputs)?;
        let outcome = Self::require_applied(
            self.plan_and_apply(
                &type_name,
                None,
                Vec::new(),
                inputs_json,
                provider_config,
                OnRequiresReplace::Ignore,
            )
            .await?,
            &type_name,
        )?;
        Ok(v0::CreateResourceResponse {
            output_properties: outcome.outputs,
            private_state: private_state::encode(private_state::Private {
                provider: outcome.apply_private,
                state: Some(outcome.new_state),
            })
            .map(v0::PrivateState),
        })
    }

    async fn update(&self, request: v0::UpdateResourceRequest) -> Result<UpdateOutcome> {
        let type_name = request.resource.type_.0.clone();
        let (new_inputs, provider_config) =
            crate::provider_config::split(request.input_properties.0)?;
        let new_inputs_json = properties_to_cty_json(new_inputs)?;
        let stored = private_state::decode(request.resource.private_state.map(|p| p.0))?;
        let prior_private = stored.provider;
        // The stored inputs carry the provider configuration the resource
        // was created with; destroy reuses it, update uses the new one.
        let (prior_inputs, _prior_provider_config) =
            crate::provider_config::split(request.resource.input_properties.0)?;
        let prior = PriorState {
            inputs: prior_inputs,
            outputs: request.resource.output_properties.map(|op| op.0),
            state: stored.state,
        };
        let outcome = self
            .plan_and_apply(
                &type_name,
                Some(prior),
                prior_private,
                new_inputs_json,
                provider_config,
                OnRequiresReplace::Report,
            )
            .await?;
        let applied = match outcome {
            // The provider's ForceNew, passed on as it is: nixops4 destroys
            // the resource and creates it again, in the order the resource's
            // `replaceOrder` asks for.
            PlanOutcome::RequiresReplace { reasons } => {
                return Ok(UpdateOutcome::RequiresReplace(
                    v0::RequiresReplaceResponse { reasons },
                ))
            }
            PlanOutcome::Applied(applied) => applied,
        };
        Ok(UpdateOutcome::Updated(v0::UpdateResourceResponse {
            output_properties: applied.outputs,
            private_state: private_state::encode(private_state::Private {
                provider: applied.apply_private,
                state: Some(applied.new_state),
            })
            .map(v0::PrivateState),
        }))
    }

    async fn destroy(
        &self,
        request: v0::DestroyResourceRequest,
    ) -> Result<v0::DestroyResourceResponse> {
        let type_name = request.resource.type_.0.clone();
        let stored = private_state::decode(request.resource.private_state.map(|p| p.0))?;
        let prior_private = stored.provider;
        // The stored inputs carry the provider configuration the resource
        // was created with; destroy reuses it, update uses the new one.
        let (prior_inputs, prior_provider_config) =
            crate::provider_config::split(request.resource.input_properties.0)?;
        let prior = PriorState {
            inputs: prior_inputs,
            outputs: request.resource.output_properties.map(|op| op.0),
            state: stored.state,
        };
        // A tfplugin destroy is Plan + Apply with `proposed_new_state`
        // = `planned_state` = `config` = cty null. The provider echoes
        // that null through to `new_state`, and any output properties
        // the provider might have returned are discarded — nixops4
        // will drop the resource from state. The `private` bytes are
        // likewise discarded: the resource is gone.
        let _ = Self::require_applied(
            self.plan_and_apply(
                &type_name,
                Some(prior),
                prior_private,
                b"null".to_vec(),
                prior_provider_config,
                OnRequiresReplace::Ignore,
            )
            .await?,
            &type_name,
        )?;
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
        provider_config: Value,
        on_requires_replace: OnRequiresReplace,
    ) -> Result<PlanOutcome> {
        let client = ProviderClient::launch(&self.provider_exe).await?;
        let result = plan_and_apply_inner(
            &client,
            type_name,
            prior,
            prior_private,
            inputs_json,
            provider_config,
            on_requires_replace,
        )
        .await;
        client.shutdown().await?;
        result
    }

    /// The outcome of a Plan that cannot be answered with a replacement.
    ///
    /// A create is planned against a null prior and a destroy is planned to
    /// null, so neither can come back asking for one; a provider that does it
    /// anyway is saying something this adapter has no reading for.
    fn require_applied(outcome: PlanOutcome, type_name: &str) -> Result<PlanApplyOutcome> {
        match outcome {
            PlanOutcome::Applied(applied) => Ok(applied),
            PlanOutcome::RequiresReplace { reasons } => anyhow::bail!(
                "provider asked for {} to be replaced ({}) where a replacement is not an answer",
                type_name,
                reasons.join(", ")
            ),
        }
    }
}

async fn plan_and_apply_inner(
    client: &ProviderClient,
    type_name: &str,
    prior: Option<PriorState>,
    prior_private: Vec<u8>,
    inputs_json: Vec<u8>,
    provider_config: Value,
    on_requires_replace: OnRequiresReplace,
) -> Result<PlanOutcome> {
    // Schema is fetched once per adapter invocation. Both the
    // projection of `new_state` onto declared computed outputs and
    // the stripping of write_only attributes from `prior_state` need
    // it — we cannot reliably identify either without the schema.
    let raw_schema = client.get_provider_schema().await?;
    let schema = ProviderSchema::from_proto(raw_schema)?;

    // cty's msgpack decoder wants total objects; nixops4 gives us only
    // what the user set. See `complete.rs`.
    let resource = schema
        .resource_schemas
        .get(type_name)
        .with_context(|| format!("no schema entry for resource type {}", type_name))?;

    let inputs_json = {
        let value: Value = serde_json::from_slice(&inputs_json)
            .with_context(|| format!("parsing inputs for {} as JSON", type_name))?;
        let value = crate::complete::complete_block(&resource.block, value, type_name)?;
        serde_json::to_vec(&value).context("serialising completed inputs")?
    };
    let provider_config = {
        let value =
            crate::complete::complete_block(&schema.provider.block, provider_config, "provider")?;
        serde_json::to_vec(&value).context("serialising provider configuration")?
    };

    let prior_state_json = match prior {
        None => b"null".to_vec(),
        Some(p) => build_prior_state_json(&schema, type_name, p.state, p.inputs, p.outputs)?,
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

    client.configure_provider(provider_config).await?;

    // Validate the resource config before Plan. Surfaces schema-shape
    // and provider-invariant errors at the earliest layer, where the
    // provider's message is most specific to the user's config — Plan
    // would either error later with a less-specific message, or (worse)
    // silently substitute defaults for the bad input.
    //
    // Not on destroy: the config is null then, and a provider checks
    // required attributes against it (terraform-plugin-sdk v2 does),
    // which would refuse every destroy. Terraform core skips
    // validation on destroy as well.
    if inputs_json != b"null" {
        client
            .validate_resource_config(type_name, inputs_json.clone())
            .await?;
    }

    // Terraform merges prior state into the config before Plan; see
    // `proposed.rs`. On create (prior null) this is the config itself.
    let proposed_json = {
        let prior_value: Value = serde_json::from_slice(&prior_state_json)
            .context("parsing prior_state before proposing")?;
        let config_value: Value =
            serde_json::from_slice(&inputs_json).context("parsing config before proposing")?;
        serde_json::to_vec(&crate::proposed::proposed_new(
            &resource.block,
            &prior_value,
            &config_value,
        ))
        .context("serialising proposed_new_state")?
    };

    let plan = client
        .plan_resource_change(
            type_name,
            prior_state_json.clone(),
            proposed_json,
            inputs_json.clone(),
            prior_private,
        )
        .await?;
    // Before the Apply: the resource has to be untouched when the adapter
    // reports that it needs replacing, because what nixops4 destroys next is
    // the resource as this request described it.
    if on_requires_replace == OnRequiresReplace::Report && !plan.requires_replace.is_empty() {
        return Ok(PlanOutcome::RequiresReplace {
            reasons: crate::tf_provider_client::requires_replace_paths(&plan.requires_replace),
        });
    }

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

    let new_state_value = crate::dynamic_value::to_json(&new_state)
        .context("decoding new_state from the provider")?;
    let new_state_json = serde_json::to_vec(&new_state_value).context("serialising new_state")?;
    let outputs = project_new_state(&schema, type_name, &new_state_json)?;
    let outputs = wrap_sensitive_outputs(&schema, type_name, outputs)?;
    Ok(PlanOutcome::Applied(PlanApplyOutcome {
        new_state: new_state_value,
        outputs: v0::OutputProperties(outputs),
        apply_private: apply.private,
    }))
}

fn properties_to_cty_json(props: Map<String, Value>) -> Result<Vec<u8>> {
    serde_json::to_vec(&Value::Object(props)).context("serialising resource properties")
}
