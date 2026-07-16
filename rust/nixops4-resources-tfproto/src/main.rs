//! CLI for `nixops4-resources-tfproto`.
//!
//! * `schema --provider-exe <path>` launches a tfplugin v6 provider and
//!   writes its schema, translated into [`ProviderSchema`], as JSON to
//!   stdout. That JSON is what `tfCommonSchemaToModule` reads from Nix.
//! * `run --provider-exe <path>` serves as a NixOps4 resource provider,
//!   translating each NixOps4 request into a tfplugin gRPC sequence
//!   against the child provider.

// Zero all heap allocations on free. See nixops4's
// `architecture/threat-model.md`.
#[global_allocator]
static ALLOC: zeroizing_alloc::ZeroAlloc<std::alloc::System> =
    zeroizing_alloc::ZeroAlloc(std::alloc::System);

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use nixops4_resources_tfproto::{
    provider::TfPluginProvider, schema::ProviderSchema, tf_provider_client::ProviderClient,
};

#[derive(Parser)]
#[command(name = "nixops4-resources-tfproto", version, about)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Launch a tfplugin provider and print its schema as JSON.
    Schema {
        /// Path to the provider binary (must speak tfplugin v6).
        #[arg(long)]
        provider_exe: String,
    },
    /// Serve as a NixOps4 resource provider driving the given tfplugin binary.
    Run {
        /// Path to the tfplugin v6 provider binary.
        #[arg(long)]
        provider_exe: String,
    },
}

fn main() -> Result<()> {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?
        .block_on(async_main())
}

async fn async_main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Command::Schema { provider_exe } => cmd_schema(&provider_exe).await,
        Command::Run { provider_exe } => cmd_run(provider_exe).await,
    }
}

async fn cmd_run(provider_exe: String) -> Result<()> {
    nixops4_resource::framework::run_main(TfPluginProvider::new(provider_exe)).await;
    Ok(())
}

async fn cmd_schema(provider_exe: &str) -> Result<()> {
    let client = ProviderClient::launch(provider_exe).await?;
    // Shutdown must happen even on error to avoid leaking the child.
    let result = async {
        let raw = client.get_provider_schema().await?;
        let schema = ProviderSchema::from_proto(raw)?;
        println!(
            "{}",
            serde_json::to_string_pretty(&schema).context("serialising schema")?
        );
        Ok::<(), anyhow::Error>(())
    }
    .await;
    client.shutdown().await?;
    result
}
