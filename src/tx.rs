use std::{env, sync::Arc, time::Duration};

use anyhow::{Result, anyhow};
use jito_json_rpc_client::jsonrpc_client::rpc_client::RpcClient as JitoRpcClient;
use solana_client::rpc_client::RpcClient;
use solana_sdk::{
    instruction::Instruction,
    signature::Keypair,
    signer::Signer,
    system_transaction,
    transaction::{Transaction, VersionedTransaction},
};
use spl_token::{amount_to_ui_amount, ui_amount_to_amount};

use tokio::time::Instant;
use tracing::{error, info};

use crate::jito::{self, get_tip_account, get_tip_value, wait_for_bundle_confirmation};

pub async fn new_signed_and_send(
    client: &RpcClient,
    keypair: &Keypair,
    instructions: Vec<Instruction>,
) -> Result<Vec<String>> {
    // send init tx
    let recent_blockhash = client.get_latest_blockhash()?;
    let txn = Transaction::new_signed_with_payer(
        &instructions,
        Some(&keypair.pubkey()),
        &vec![&*keypair],
        recent_blockhash,
    );

    if env::var("TX_SIMULATE").ok() == Some("true".to_string()) {
        let simulate_result = client.simulate_transaction(&txn)?;
        if let Some(logs) = simulate_result.value.logs {
            for log in logs {
                info!("{}", log);
            }
        }
        return match simulate_result.value.err {
            Some(err) => Err(anyhow!("{}", err)),
            None => Ok(vec![]),
        };
    }

    let start_time = Instant::now();
    // jito
    let tip_account = get_tip_account().await?;
    // jito tip, the upper limit is 0.1
    let mut tip = get_tip_value().await?;
    tip = tip.min(0.1);
    let tip_lamports = ui_amount_to_amount(tip, spl_token::native_mint::DECIMALS);
    info!(
        "tip account: {}, tip(sol): {}, lamports: {}",
        tip_account, tip, tip_lamports
    );

    let jito_client = Arc::new(JitoRpcClient::new(format!(
        "{}/api/v1/bundles",
        jito::BLOCK_ENGINE_URL.to_string()
    )));
    // tip tx
    let mut bundle: Vec<VersionedTransaction> = vec![];
    bundle.push(VersionedTransaction::from(txn));
    bundle.push(VersionedTransaction::from(system_transaction::transfer(
        &keypair,
        &tip_account,
        tip_lamports,
        recent_blockhash,
    )));
    let bundle_id = jito_client.send_bundle(&bundle).await?;
    info!("bundle_id: {}", bundle_id);

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
        bundle_id,
        Duration::from_millis(1000),
        Duration::from_secs(10),
    )
    .await?;

    info!("tx elapsed: {:?}", start_time.elapsed());
    Ok(txs)
}

pub async fn send_transaction_with_tip(
    client: &RpcClient,
    keypair: &Keypair,
    versioned_transactions: Vec<VersionedTransaction>,
    tip_lamports: u64,
    wait_for_confirmation: bool,
) -> Result<Vec<String>> {
    // send init tx
    let recent_blockhash = client.get_latest_blockhash()?;

    let start_time = Instant::now();
    // jito
    let tip_account = get_tip_account().await?;
    let tip = amount_to_ui_amount(tip_lamports, spl_token::native_mint::DECIMALS);
    info!(
        "tip account: {}, tip(sol): {}, lamports: {}",
        tip_account, tip, tip_lamports
    );

    let jito_client = Arc::new(JitoRpcClient::new(format!(
        "{}/api/v1/bundles",
        jito::BLOCK_ENGINE_URL.to_string()
    )));
    // tip tx
    let mut bundle: Vec<VersionedTransaction> = vec![];
    for versioned_transaction in versioned_transactions {
        let signed_versioned_transaction =
            VersionedTransaction::try_new(versioned_transaction.message, &[&keypair])?;
        bundle.push(signed_versioned_transaction);
    }
    bundle.push(VersionedTransaction::from(system_transaction::transfer(
        &keypair,
        &tip_account,
        tip_lamports,
        recent_blockhash,
    )));
    let bundle_id = jito_client.send_bundle(&bundle).await?;
    info!("bundle_id: {}", bundle_id);

    let txs = if wait_for_confirmation {
        wait_for_bundle_confirmation(
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
            bundle_id,
            Duration::from_millis(200),
            Duration::from_secs(1),
        )
        .await?
    } else {
        vec![]
    };

    info!("tx elapsed: {:?}", start_time.elapsed());
    Ok(txs)
}
