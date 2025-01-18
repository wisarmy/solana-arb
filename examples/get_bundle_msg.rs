use anyhow::Result;
use clap::Parser;
use jito_json_rpc_client::jsonrpc_client::rpc_client::RpcClient as JitoRpcClient;
use solana_arb::{
    jito::{self, wait_for_bundle_confirmation},
    logger,
};
use std::{sync::Arc, time::Duration};
use tracing::{error, info};

#[derive(Parser)]
#[command(author, version, about, long_about = None)]
struct Cli {
    /// Bundle ID to query
    bundle_id: String,
}

// Use: cargo r --example get_bundle_msg <bundle_id>
#[tokio::main]
async fn main() -> Result<()> {
    // Load environment variables
    dotenvy::dotenv().ok();
    // Initialize logger
    logger::init(true);

    // Parse command line arguments
    let cli = Cli::parse();

    let jito_client = Arc::new(JitoRpcClient::new(format!(
        "{}/api/v1/bundles",
        jito::BLOCK_ENGINE_URL.to_string()
    )));

    info!("Querying bundle ID: {}", cli.bundle_id);

    let txs = wait_for_bundle_confirmation(
        move |id: String| {
            let client = Arc::clone(&jito_client);
            async move {
                let response = client.get_bundle_statuses(&[id]).await;
                let statuses = response.inspect_err(|err| {
                    error!("Error fetching bundle status: {:?}", err);
                })?;
                Ok(statuses.value)
            }
        },
        cli.bundle_id,
        Duration::from_millis(30000),
        Duration::from_secs(30),
        true,
    )
    .await?;

    info!("Transaction hashes: {:?}", txs);

    Ok(())
}
