use anyhow::Result;
use rand::seq::SliceRandom;
use solana_client::{self, rpc_client::RpcClient};
use solana_sdk::signature::Keypair;
use std::{env, sync::Arc};
use tracing::debug;

pub mod arb;
pub mod dex;
pub mod jito;
pub mod logger;
pub mod token;
pub mod tx;

pub fn get_random_rpc_url() -> Result<String> {
    let cluster_urls = env::var("RPC_ENDPOINTS")?
        .split(",")
        .map(|s| s.trim().to_string())
        .collect::<Vec<String>>();

    let random_url = cluster_urls
        .choose(&mut rand::thread_rng())
        .expect("No RPC endpoints configured")
        .clone();
    debug!("Choose rpc: {}", random_url);
    return Ok(random_url);
}

pub fn get_rpc_client() -> Result<Arc<RpcClient>> {
    let random_url = get_random_rpc_url()?;
    let client = RpcClient::new(random_url);
    return Ok(Arc::new(client));
}

pub fn get_payer() -> Result<Arc<Keypair>> {
    let wallet = Keypair::from_base58_string(&env::var("PRIVATE_KEY")?);
    return Ok(Arc::new(wallet));
}

#[cfg(test)]
mod tests {
    #[ctor::ctor]
    fn init() {
        crate::logger::init(true);
        dotenvy::dotenv().ok();
    }
}
