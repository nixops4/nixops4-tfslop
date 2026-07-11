//! Spawn a tfplugin provider and connect to it over gRPC.

use anyhow::{bail, Context, Result};
use hyper_util::rt::TokioIo;
use std::process::Stdio;
use std::sync::Mutex;
use std::time::Duration;
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
    // Wrapped so we can peek at the child's exit status from `&self`
    // RPC methods after a transport error. try_wait is a quick fcntl-
    // style probe; std Mutex is enough because we never hold the lock
    // across an await.
    process: Mutex<Child>,
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

        Ok(ProviderClient {
            process: Mutex::new(process),
            channel,
        })
    }

    /// SIGKILL the provider and wait for it to exit.
    ///
    /// The channel is dropped first so the server observes EOF cleanly.
    pub async fn shutdown(self) -> Result<()> {
        drop(self.channel);
        // A poisoned mutex here means we panicked while inspecting the
        // child — the child is still ours to reap either way.
        let mut process = match self.process.into_inner() {
            Ok(p) => p,
            Err(poison) => poison.into_inner(),
        };
        let _ = process.start_kill();
        let _ = process.wait().await;
        Ok(())
    }

    /// Wait briefly for the child to reap and, if it has exited,
    /// decorate the error with the exit status so the user can tell
    /// "provider crashed" from "transport hiccup". A transport-level
    /// error whose child is still alive stays as-is.
    async fn decorate_with_child_exit(&self, err: anyhow::Error) -> anyhow::Error {
        // The OS may not have marked the process as reaped by the
        // time tonic surfaces the transport error. A short pause makes
        // detection reliable without adding noticeable latency.
        tokio::time::sleep(Duration::from_millis(50)).await;
        let mut guard = match self.process.lock() {
            Ok(g) => g,
            Err(poison) => poison.into_inner(),
        };
        match guard.try_wait() {
            Ok(Some(status)) => {
                err.context(format!("provider process exited with status {}", status))
            }
            _ => err,
        }
    }

    /// Turn a `tonic::Status` (transport-level) into an anyhow error
    /// tagged with the RPC name, and, if the child has since exited,
    /// with its status.
    async fn on_transport_error(&self, rpc: &str, e: tonic::Status) -> anyhow::Error {
        let base = anyhow::Error::from(e).context(format!("calling {}", rpc));
        self.decorate_with_child_exit(base).await
    }
    fn provider(&self) -> tfplugin6::provider_client::ProviderClient<Channel> {
        tfplugin6::provider_client::ProviderClient::new(self.channel.clone())
    }

    /// Call `GetProviderSchema` and return the raw response.
    ///
    /// Any provider-side [`tfplugin6::Diagnostic`] with error severity in
    /// the response is surfaced as an `Err` here.
    pub async fn get_provider_schema(&self) -> Result<tfplugin6::get_provider_schema::Response> {
        let response = match self
            .provider()
            .get_provider_schema(tonic::Request::new(
                tfplugin6::get_provider_schema::Request {},
            ))
            .await
        {
            Ok(r) => r.into_inner(),
            Err(e) => return Err(self.on_transport_error("GetProviderSchema", e).await),
        };
        check_diagnostics(&response.diagnostics)?;
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
        let response = match self
            .provider()
            .configure_provider(tonic::Request::new(req))
            .await
        {
            Ok(r) => r.into_inner(),
            Err(e) => return Err(self.on_transport_error("ConfigureProvider", e).await),
        };
        check_diagnostics(&response.diagnostics)
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
        let response = match self
            .provider()
            .plan_resource_change(tonic::Request::new(req))
            .await
        {
            Ok(r) => r.into_inner(),
            Err(e) => return Err(self.on_transport_error("PlanResourceChange", e).await),
        };
        check_diagnostics(&response.diagnostics)?;
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
        let response = match self
            .provider()
            .apply_resource_change(tonic::Request::new(req))
            .await
        {
            Ok(r) => r.into_inner(),
            Err(e) => return Err(self.on_transport_error("ApplyResourceChange", e).await),
        };
        check_diagnostics(&response.diagnostics)?;
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
///
/// Severities are matched via `try_from` on the generated enum so any
/// value we don't recognise surfaces explicitly instead of being
/// silently filtered out. Multiple errors are aggregated on separate
/// lines: a `; `-joined single line collapses per-diagnostic
/// structure and, worse, that delimiter can occur in payloads.
fn check_diagnostics(diagnostics: &[tfplugin6::Diagnostic]) -> Result<()> {
    use tfplugin6::diagnostic::Severity;

    let mut errors: Vec<String> = Vec::new();
    for d in diagnostics {
        match Severity::try_from(d.severity) {
            Ok(Severity::Invalid) => {
                // Placeholder value; treat as absent.
            }
            Ok(Severity::Warning) => {
                if d.detail.is_empty() {
                    eprintln!("provider warning: {}", d.summary);
                } else {
                    eprintln!("provider warning: {}: {}", d.summary, d.detail);
                }
            }
            Ok(Severity::Error) => errors.push(format_diagnostic(d)),
            // Unknown severity: don't drop it — providers using a
            // newer variant would otherwise go unheard.
            Err(_) => errors.push(format!(
                "unknown severity {}: {}",
                d.severity,
                format_diagnostic(d)
            )),
        }
    }

    match errors.len() {
        0 => Ok(()),
        1 => bail!("{}", errors.remove(0)),
        _ => {
            let numbered: Vec<String> = errors
                .into_iter()
                .enumerate()
                .map(|(i, msg)| format!("  {}. {}", i + 1, msg))
                .collect();
            bail!(
                "provider reported {} errors:\n{}",
                numbered.len(),
                numbered.join("\n")
            )
        }
    }
}

fn format_diagnostic(d: &tfplugin6::Diagnostic) -> String {
    let body = if d.detail.is_empty() {
        d.summary.clone()
    } else {
        format!("{}: {}", d.summary, d.detail)
    };
    match d.attribute.as_ref().and_then(format_attribute_path) {
        Some(path) => format!("at {}: {}", path, body),
        None => body,
    }
}

/// Format a tfplugin `AttributePath` as a Nix-config-shaped string:
/// `foo.bar[0].baz`, `tags["env"]`. Returns `None` for an empty path
/// so an absent path doesn't produce a stray `at : ` prefix.
fn format_attribute_path(p: &tfplugin6::AttributePath) -> Option<String> {
    use tfplugin6::attribute_path::step::Selector;
    if p.steps.is_empty() {
        return None;
    }
    let mut out = String::new();
    for (i, step) in p.steps.iter().enumerate() {
        match &step.selector {
            Some(Selector::AttributeName(name)) => {
                if i > 0 {
                    out.push('.');
                }
                out.push_str(name);
            }
            Some(Selector::ElementKeyInt(k)) => {
                out.push_str(&format!("[{}]", k));
            }
            Some(Selector::ElementKeyString(k)) => {
                // Escape embedded quotes so `tags["a\"b"]` stays parseable.
                let escaped = k.replace('\\', r"\\").replace('"', r#"\""#);
                out.push_str(&format!("[\"{}\"]", escaped));
            }
            None => {
                // Malformed step from a provider — surface as `<?>`
                // so we don't just silently drop it.
                out.push_str("<?>");
            }
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tfplugin6::diagnostic::Severity;

    fn diag(sev: i32, summary: &str, detail: &str) -> tfplugin6::Diagnostic {
        tfplugin6::Diagnostic {
            severity: sev,
            summary: summary.to_string(),
            detail: detail.to_string(),
            attribute: None,
        }
    }

    // #71: An unrecognised severity value must not be silently
    // dropped. Today `severity == Error as i32` silently ignores
    // anything that isn't Error/Warning; if the proto grows Info or
    // Fatal, providers using it would go unheard.
    #[test]
    fn unknown_severity_is_reported() {
        let diagnostics = vec![diag(99, "mystery", "unexpected severity")];
        let result = check_diagnostics(&diagnostics);
        assert!(
            result.is_err(),
            "unknown severity {} should not be silently dropped",
            99
        );
        let msg = format!("{}", result.unwrap_err());
        assert!(
            msg.contains("99") && msg.contains("mystery"),
            "error should identify the severity code and the message; got: {msg}"
        );
    }

    // #72: Multiple errors must not collapse into one flat string
    // joined with "; ". A payload can legitimately contain that
    // sequence, and per-diagnostic structure is lost.
    #[test]
    fn multiple_errors_preserve_structure() {
        let diagnostics = vec![
            diag(Severity::Error as i32, "first", "detail one"),
            diag(Severity::Error as i32, "second; with; semicolons", ""),
        ];
        let result = check_diagnostics(&diagnostics);
        let msg = format!("{}", result.unwrap_err());
        // The exact format is a design choice, but two constraints:
        // (a) both messages are present; (b) they are separated in a
        // way a reader can tell apart even when payloads contain "; ".
        assert!(msg.contains("first"), "missing first error: {msg}");
        assert!(
            msg.contains("second; with; semicolons"),
            "missing second error verbatim: {msg}"
        );
        assert!(
            msg.contains('\n'),
            "multiple errors should be on separate lines, not joined by delimiter: {msg}"
        );
    }

    // #73: `check_diagnostics(...).context("provider returned
    // diagnostics")?` is the only caller pattern, so the wrapper
    // string ends up above every diagnostic in the user's view.
    // Assert the *inner* error does not lead with that wrapper — the
    // caller is the one that decides context, not the check.
    #[test]
    fn error_does_not_include_wrapper_phrase() {
        let diagnostics = vec![diag(Severity::Error as i32, "bad thing", "specifics")];
        let result = check_diagnostics(&diagnostics);
        let msg = format!("{}", result.unwrap_err());
        assert!(
            !msg.contains("provider returned diagnostics"),
            "check_diagnostics must not embed the caller's context: {msg}"
        );
        // The user-facing message should lead with the provider's
        // own summary, not a synthetic prefix.
        assert!(
            msg.starts_with("bad thing"),
            "error should lead with the provider's summary; got: {msg}"
        );
    }

    #[test]
    fn single_error_is_terse() {
        let diagnostics = vec![diag(Severity::Error as i32, "boom", "detail")];
        let result = check_diagnostics(&diagnostics);
        let msg = format!("{}", result.unwrap_err());
        assert_eq!(msg, "boom: detail");
    }

    fn attr_step(name: &str) -> tfplugin6::attribute_path::Step {
        tfplugin6::attribute_path::Step {
            selector: Some(tfplugin6::attribute_path::step::Selector::AttributeName(
                name.to_string(),
            )),
        }
    }

    fn int_step(i: i64) -> tfplugin6::attribute_path::Step {
        tfplugin6::attribute_path::Step {
            selector: Some(tfplugin6::attribute_path::step::Selector::ElementKeyInt(i)),
        }
    }

    fn string_step(k: &str) -> tfplugin6::attribute_path::Step {
        tfplugin6::attribute_path::Step {
            selector: Some(tfplugin6::attribute_path::step::Selector::ElementKeyString(
                k.to_string(),
            )),
        }
    }

    fn diag_at(
        sev: i32,
        summary: &str,
        steps: Vec<tfplugin6::attribute_path::Step>,
    ) -> tfplugin6::Diagnostic {
        tfplugin6::Diagnostic {
            severity: sev,
            summary: summary.to_string(),
            detail: String::new(),
            attribute: Some(tfplugin6::AttributePath { steps }),
        }
    }

    // #49: `Diagnostic.attribute` names the config path an error is
    // about (`settings.token`, `rules[0].name`, `tags["env"]`).
    // Dropping it makes deep errors unactionable — the user knows
    // *what* is wrong but not *where*.
    #[test]
    fn error_includes_attribute_path() {
        let diagnostics = vec![diag_at(
            Severity::Error as i32,
            "value is malformed",
            vec![attr_step("settings"), attr_step("token")],
        )];
        let result = check_diagnostics(&diagnostics);
        let msg = format!("{}", result.unwrap_err());
        assert!(
            msg.contains("settings.token"),
            "error should name the attribute path settings.token; got: {msg}"
        );
    }

    #[test]
    fn error_path_formats_list_index() {
        let diagnostics = vec![diag_at(
            Severity::Error as i32,
            "invalid rule",
            vec![attr_step("rules"), int_step(2), attr_step("name")],
        )];
        let msg = format!("{}", check_diagnostics(&diagnostics).unwrap_err());
        assert!(
            msg.contains("rules[2].name"),
            "error should name path rules[2].name; got: {msg}"
        );
    }

    #[test]
    fn error_path_formats_string_key() {
        let diagnostics = vec![diag_at(
            Severity::Error as i32,
            "bad tag",
            vec![attr_step("tags"), string_step("env")],
        )];
        let msg = format!("{}", check_diagnostics(&diagnostics).unwrap_err());
        assert!(
            msg.contains(r#"tags["env"]"#),
            r#"error should name path tags["env"]; got: {msg}"#
        );
    }

    // A missing / empty attribute path must not produce noise like
    // "at : summary".
    #[test]
    fn error_without_path_has_no_prefix_artifact() {
        let diagnostics = vec![diag_at(Severity::Error as i32, "no path here", vec![])];
        let msg = format!("{}", check_diagnostics(&diagnostics).unwrap_err());
        assert_eq!(msg, "no path here");
    }

    #[test]
    fn warning_only_is_ok() {
        let diagnostics = vec![diag(Severity::Warning as i32, "heads up", "advisory")];
        let result = check_diagnostics(&diagnostics);
        assert!(result.is_ok(), "warnings alone must not cause an Err");
    }

    #[test]
    fn no_diagnostics_is_ok() {
        let result = check_diagnostics(&[]);
        assert!(result.is_ok());
    }

    // Severity::Invalid (0) is the "no severity set" placeholder in
    // the proto; treat it as a no-op like an absent diagnostic.
    #[test]
    fn invalid_severity_is_ignored() {
        let diagnostics = vec![diag(Severity::Invalid as i32, "nothing", "")];
        let result = check_diagnostics(&diagnostics);
        assert!(
            result.is_ok(),
            "Severity::Invalid should be ignored, not treated as an error"
        );
    }
}
