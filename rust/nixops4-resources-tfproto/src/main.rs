//! CLI for `nixops4-resources-tfproto`.
//!
//! `schema --provider-exe <path>` launches a tfplugin v6 provider and
//! writes its schema, translated into [`ProviderSchema`], as JSON to
//! stdout. That JSON is what `tfCommonSchemaToModule` reads from Nix.

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use nixops4_resources_tfproto::{schema::ProviderSchema, tf_provider_client::ProviderClient};

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
    }
}

async fn cmd_schema(provider_exe: &str) -> Result<()> {
    let client = ProviderClient::launch(provider_exe).await?;
    // Shutdown must happen even on error to avoid leaking the child.
    let result = async {
        let raw = client.get_provider_schema().await?;
        let schema = ProviderSchema::from_wire(raw)?;
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
