//! Spawn a tfplugin provider and connect to it over gRPC.

use anyhow::{bail, Context, Result};
use hyper_util::rt::TokioIo;
use std::process::Stdio;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::net::UnixStream;
use tokio::process::{Child, Command};
use tonic::transport::{Channel, Endpoint, Uri};
use tower::service_fn;

use crate::tfplugin6;

use super::handshake;

/// go-plugin's magic cookie, expected by every Terraform / OpenTofu
/// provider. Launches that don't set this in the environment are rejected.
const TF_PLUGIN_MAGIC_COOKIE_VALUE: &str =
    "d602bf8f470bc67ca7faa0386276bbdd4330efaf76d1a219cb4d6991ca9872b2";

/// A live provider process and the gRPC channel talking to it.
///
/// Dropping this struct does not kill the child process. Callers must
/// call [`ProviderClient::shutdown`] to release it.
pub struct ProviderClient {
    process: Child,
    channel: Channel,
}

impl ProviderClient {
    /// Spawn `provider_path`, read the handshake line from its stdout,
    /// and connect over gRPC.
    pub async fn launch(provider_path: &str) -> Result<Self> {
        let mut cmd = Command::new(provider_path);
        cmd.env("PLUGIN_PROTOCOL_VERSIONS", "6")
            .env("TF_PLUGIN_MAGIC_COOKIE", TF_PLUGIN_MAGIC_COOKIE_VALUE)
            .env("PLUGIN_MIN_PORT", "12000")
            .env("PLUGIN_MAX_PORT", "13000")
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit());

        let mut process = cmd
            .spawn()
            .with_context(|| format!("spawning provider {}", provider_path))?;

        let stdout = process
            .stdout
            .take()
            .context("provider stdout not captured")?;
        let mut reader = BufReader::new(stdout);
        let mut line = String::new();
        reader
            .read_line(&mut line)
            .await
            .context("reading handshake line from provider")?;
        if line.is_empty() {
            bail!("provider exited before writing a handshake line");
        }

        let info = handshake::parse(&line)?;
        handshake::validate(&info)?;

        let channel = match info.network_type.as_str() {
            "tcp" => Endpoint::from_shared(format!("http://{}", info.network_address))?
                .connect()
                .await
                .context("connecting to provider over TCP")?,
            "unix" => {
                let path = info.network_address.clone();
                // tonic reaches the socket through the service_fn connector;
                // the URI is a required placeholder.
                Endpoint::try_from("http://[::]:50051")?
                    .connect_with_connector(service_fn(move |_: Uri| {
                        let path = path.clone();
                        async move {
                            let s = UnixStream::connect(path).await.map_err(|e| {
                                Box::new(e) as Box<dyn std::error::Error + Send + Sync>
                            })?;
                            Ok::<_, Box<dyn std::error::Error + Send + Sync>>(TokioIo::new(s))
                        }
                    }))
                    .await
                    .context("connecting to provider over unix socket")?
            }
            other => bail!("unsupported network type: {}", other),
        };

        Ok(ProviderClient { process, channel })
    }

    /// SIGKILL the provider and wait for it to exit.
    ///
    /// The channel is dropped first so the server observes EOF cleanly.
    pub async fn shutdown(mut self) -> Result<()> {
        drop(self.channel);
        let _ = self.process.start_kill();
        let _ = self.process.wait().await;
        Ok(())
    }
    fn provider(&self) -> tfplugin6::provider_client::ProviderClient<Channel> {
        tfplugin6::provider_client::ProviderClient::new(self.channel.clone())
    }

    /// Call `GetProviderSchema` and return the raw response.
    ///
    /// Any provider-side [`tfplugin6::Diagnostic`] with error severity in
    /// the response is surfaced as an `Err` here.
    pub async fn get_provider_schema(&self) -> Result<tfplugin6::get_provider_schema::Response> {
        let response = self
            .provider()
            .get_provider_schema(tonic::Request::new(
                tfplugin6::get_provider_schema::Request {},
            ))
            .await
            .context("calling GetProviderSchema")?
            .into_inner();
        check_diagnostics(&response.diagnostics).context("provider returned diagnostics")?;
        Ok(response)
    }

    /// Call `ConfigureProvider` with the given raw configuration bytes.
    /// A provider that takes no configuration accepts a `null` go-cty JSON
    /// (`b"null"`) here.
    pub async fn configure_provider(&self, config_json: Vec<u8>) -> Result<()> {
        let req = tfplugin6::configure_provider::Request {
            terraform_version: "0.0.0-nixops4-tfproto".to_string(),
            config: Some(tfplugin6::DynamicValue {
                msgpack: Vec::new(),
                json: config_json,
            }),
            client_capabilities: None,
        };
        let response = self
            .provider()
            .configure_provider(tonic::Request::new(req))
            .await
            .context("calling ConfigureProvider")?
            .into_inner();
        check_diagnostics(&response.diagnostics).context("provider returned diagnostics")
    }

    /// Call `PlanResourceChange` and return the raw response.
    ///
    /// `proposed_new_state_json` is the go-cty JSON encoding of what the
    /// caller wants the resource to become. For a create operation,
    /// `prior_state_json` should be `b"null"`.
    pub async fn plan_resource_change(
        &self,
        type_name: &str,
        prior_state_json: Vec<u8>,
        proposed_new_state_json: Vec<u8>,
        config_json: Vec<u8>,
    ) -> Result<tfplugin6::plan_resource_change::Response> {
        let req = tfplugin6::plan_resource_change::Request {
            type_name: type_name.to_string(),
            prior_state: Some(dv(prior_state_json)),
            proposed_new_state: Some(dv(proposed_new_state_json)),
            config: Some(dv(config_json)),
            prior_private: Vec::new(),
            provider_meta: None,
            client_capabilities: None,
            prior_identity: None,
        };
        let response = self
            .provider()
            .plan_resource_change(tonic::Request::new(req))
            .await
            .context("calling PlanResourceChange")?
            .into_inner();
        check_diagnostics(&response.diagnostics).context("provider returned diagnostics")?;
        Ok(response)
    }

    /// Call `ApplyResourceChange` and return the raw response.
    pub async fn apply_resource_change(
        &self,
        type_name: &str,
        prior_state_json: Vec<u8>,
        planned_state: tfplugin6::DynamicValue,
        planned_private: Vec<u8>,
        config_json: Vec<u8>,
    ) -> Result<tfplugin6::apply_resource_change::Response> {
        let req = tfplugin6::apply_resource_change::Request {
            type_name: type_name.to_string(),
            prior_state: Some(dv(prior_state_json)),
            planned_state: Some(planned_state),
            config: Some(dv(config_json)),
            planned_private,
            provider_meta: None,
            planned_identity: None,
        };
        let response = self
            .provider()
            .apply_resource_change(tonic::Request::new(req))
            .await
            .context("calling ApplyResourceChange")?
            .into_inner();
        check_diagnostics(&response.diagnostics).context("provider returned diagnostics")?;
        Ok(response)
    }
}

/// Build a JSON-encoded `DynamicValue` from raw go-cty JSON bytes.
fn dv(json: Vec<u8>) -> tfplugin6::DynamicValue {
    tfplugin6::DynamicValue {
        msgpack: Vec::new(),
        json,
    }
}

/// Aggregate error-severity diagnostics into an `Err`; log warnings.
///
/// tfplugin providers use warnings for deprecation notices and non-
/// fatal advisories. Losing them silently defeats the point. Nixops4
/// captures the adapter's stderr, so writing there is enough to
/// surface warnings to the user without needing a dedicated channel.
fn check_diagnostics(diagnostics: &[tfplugin6::Diagnostic]) -> Result<()> {
    for d in diagnostics {
        if d.severity == tfplugin6::diagnostic::Severity::Warning as i32 {
            if d.detail.is_empty() {
                eprintln!("provider warning: {}", d.summary);
            } else {
                eprintln!("provider warning: {}: {}", d.summary, d.detail);
            }
        }
    }
    let errors: Vec<String> = diagnostics
        .iter()
        .filter(|d| d.severity == tfplugin6::diagnostic::Severity::Error as i32)
        .map(|d| {
            if d.detail.is_empty() {
                d.summary.clone()
            } else {
                format!("{}: {}", d.summary, d.detail)
            }
        })
        .collect();
    if !errors.is_empty() {
        bail!("{}", errors.join("; "));
    }
    Ok(())
}
