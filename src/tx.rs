use std::{env, sync::Arc, time::Duration};

use anyhow::{Result, anyhow};
use jito_json_rpc_client::jsonrpc_client::rpc_client::RpcClient as JitoRpcClient;
use solana_client::rpc_client::RpcClient;
use solana_sdk::{
    address_lookup_table::{AddressLookupTableAccount, state::AddressLookupTable},
    instruction::Instruction,
    message::{VersionedMessage, v0},
    pubkey::Pubkey,
    signature::Keypair,
    signer::Signer,
    system_instruction, system_transaction,
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
        true,
    )
    .await?;

    info!("tx elapsed: {:?}", start_time.elapsed());
    Ok(txs)
}

pub fn get_tip_instruction(
    from_pubkey: &Pubkey,
    tip_account: &Pubkey,
    tip_lamports: u64,
) -> Instruction {
    let tip = amount_to_ui_amount(tip_lamports, spl_token::native_mint::DECIMALS);
    info!(
        "💎 tip account: {}, tip(sol): {}, lamports: {}",
        tip_account, tip, tip_lamports
    );
    system_instruction::transfer(from_pubkey, &tip_account, tip_lamports)
}

pub async fn send_versioned_transaction(
    client: &RpcClient,
    keypair: &Keypair,
    versioned_transaction: VersionedTransaction,
    wait_for_confirmation: bool,
) -> Result<Vec<String>> {
    // TX_SIMULATE
    if env::var("TX_SIMULATE").ok() == Some("true".to_string()) {
        let signed_versioned_transaction =
            VersionedTransaction::try_new(versioned_transaction.message, &[&keypair])?;
        let simulate_result = client
            .simulate_transaction(&signed_versioned_transaction)
            .inspect_err(|err| {
                println!("err: {}", err);
            })?;
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
    let jito_client = Arc::new(JitoRpcClient::new(format!(
        "{}/api/v1/bundles",
        jito::BLOCK_ENGINE_URL.to_string()
    )));
    let mut bundle: Vec<VersionedTransaction> = vec![];
    // sign tx
    let signed_versioned_transaction =
        VersionedTransaction::try_new(versioned_transaction.message, &[&keypair])?;
    bundle.push(signed_versioned_transaction);

    let bundle_id = jito_client.send_bundle(&bundle).await?;
    info!("📦 bundle_id: {}", bundle_id);

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
            Duration::from_millis(1000),
            Duration::from_secs(5),
            false,
        )
        .await?
    } else {
        vec![]
    };

    info!("tx elapsed: {:?}", start_time.elapsed());
    Ok(txs)
}

pub fn create_tx_with_address_table_lookup(
    client: &RpcClient,
    instructions: &mut Vec<Instruction>,
    address_lookup_table_keys: &Vec<Pubkey>,
    payer: &Keypair,
) -> Result<VersionedTransaction> {
    let raw_accounts = client.get_multiple_accounts(&address_lookup_table_keys)?;

    let address_lookup_table_accounts = address_lookup_table_keys
        .iter()
        .zip(raw_accounts.iter())
        .filter_map(|(key, account_opt)| {
            account_opt.as_ref().and_then(|account| {
                AddressLookupTable::deserialize(&account.data)
                    .map(|lookup_table| AddressLookupTableAccount {
                        key: *key,
                        addresses: lookup_table.addresses.to_vec(),
                    })
                    .ok()
            })
        })
        .collect::<Vec<AddressLookupTableAccount>>();

    let blockhash = client.get_latest_blockhash()?;
    let tx = VersionedTransaction::try_new(
        VersionedMessage::V0(v0::Message::try_compile(
            &payer.pubkey(),
            &instructions,
            &address_lookup_table_accounts,
            blockhash,
        )?),
        &[payer],
    )?;

    Ok(tx)
}
