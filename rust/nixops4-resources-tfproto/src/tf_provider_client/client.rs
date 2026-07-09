//! Spawn a tfplugin provider and connect to it over gRPC.

use anyhow::{bail, Context, Result};
use hyper_util::rt::TokioIo;
use std::process::Stdio;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::net::UnixStream;
use tokio::process::{Child, Command};
use tonic::transport::{Channel, Endpoint, Uri};
use tower::service_fn;

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
}
