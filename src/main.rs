use std::collections::HashMap;
use std::env;

use anyhow::Result;
use clap::{Parser, Subcommand};
use jupiter_swap_api_client::transaction_config::ComputeUnitPriceMicroLamports;
use jupiter_swap_api_client::{
    JupiterSwapApiClient, quote::QuoteRequest, swap::SwapRequest,
    transaction_config::TransactionConfig,
};
use solana_arb::dex::Dex;
use solana_arb::token::get_mint;
use solana_arb::tx::create_tx_with_address_table_lookup;
use solana_arb::{arb, get_payer, get_rpc_client, jito, logger, tx};
use solana_sdk::pubkey::Pubkey;
use solana_sdk::signature::Keypair;
use solana_sdk::signer::Signer;
use solana_sdk::transaction::VersionedTransaction;
use spl_token::{amount_to_ui_amount, ui_amount_to_amount};
use tracing::{debug, info, warn};

#[derive(Parser)]
#[command(author, version, about, long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    Swap {
        mint: Pubkey,
        #[clap(help = "Swap direction: [buy, sell]")]
        direction: String,
        #[clap(help = "WSOL ui amount for swap")]
        amount_in: f64,
    },

    Arb {
        mint: Pubkey,
        #[clap(help = "WSOL ui amount for arbitrage")]
        amount_in: f64,
        #[arg(
            long,
            help = "Interval between each arbitrage attempt in seconds",
            default_value_t = 1
        )]
        interval: u64,
        #[arg(
            long,
            help = "Minimum profit in SOL to trigger arbitrage",
            default_value_t = 0.0001
        )]
        min_profit: f64,
        #[arg(long, help = "Jupiter partner referral fee", default_value_t = 0.0)]
        partner_fee: f64,

        #[arg(long, help = "Wait for confirmation", default_value_t = false)]
        wait_for_confirmation: bool,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    dotenvy::dotenv().ok();
    let cli = Cli::parse();
    logger::init(true);

    let rpc_client = get_rpc_client()?;
    let payer = get_payer()?;

    let api_base_url = env::var("JUP_QUOTE_API").unwrap_or("https://quote-api.jup.ag/v6".into());
    info!("Using jupiter quote api url: {}", api_base_url);
    let jupiter_extra_args: Option<HashMap<String, String>> =
        env::var("JUP_QUOTE_API_KEY").ok().map(|api_key| {
            let mut args = HashMap::new();
            args.insert("api_key".to_string(), api_key);
            args
        });
    let jupiter_swap_api_client = JupiterSwapApiClient::new(api_base_url);

    match &cli.command {
        Commands::Swap {
            mint,
            direction,
            amount_in,
        } => {
            info!(
                "mint: {}, direction: {}, amount_in: {}",
                mint, direction, amount_in
            );

            let native_mint = spl_token::native_mint::id();

            let (token_in, token_out) = match direction.as_str() {
                "buy" => (native_mint, *mint),
                "sell" => (*mint, native_mint),
                _ => panic!("Invalid direction"),
            };
            let in_mint = get_mint(&rpc_client, &token_in)?;

            let quote_request = QuoteRequest {
                amount: ui_amount_to_amount(*amount_in, in_mint.decimals),
                input_mint: token_in,
                output_mint: token_out,
                dexes: Some("Raydium,Meteora DLMM,Whirlpool".into()),
                slippage_bps: 500,
                quote_args: jupiter_extra_args.clone(),
                ..QuoteRequest::default()
            };
            // GET /quote
            let quote_response = jupiter_swap_api_client.quote(&quote_request).await.unwrap();
            println!("{quote_response:#?}");
            let mut tx_config = TransactionConfig::default();
            tx_config.wrap_and_unwrap_sol = true;
            tx_config.compute_unit_price_micro_lamports =
                Some(ComputeUnitPriceMicroLamports::MicroLamports(50000));
            // POST /swap
            let swap_response = jupiter_swap_api_client
                .swap(
                    &SwapRequest {
                        user_public_key: payer.pubkey(),
                        quote_response: quote_response.clone(),
                        config: TransactionConfig::default(),
                    },
                    jupiter_extra_args,
                )
                .await
                .unwrap();

            println!("Raw tx len: {}", swap_response.swap_transaction.len());
            println!("Raw tx: {:?}", swap_response);

            let versioned_transaction: VersionedTransaction =
                bincode::deserialize(&swap_response.swap_transaction).unwrap();

            let signed_versioned_transaction =
                VersionedTransaction::try_new(versioned_transaction.message, &[&payer]).unwrap();
            match rpc_client.send_and_confirm_transaction(&signed_versioned_transaction) {
                Ok(signer) => {
                    println!("signer: {signer}");
                }
                Err(err) => {
                    println!("Error: {err}");
                }
            }
        }

        Commands::Arb {
            mint,
            amount_in,
            interval,
            min_profit,
            partner_fee,
            wait_for_confirmation,
        } => {
            info!(
                "mint: {}, amount_in: {}, interval: {}s, min_profit: {} SOL",
                mint, amount_in, interval, min_profit
            );
            let min_profit_lamports = ui_amount_to_amount(*min_profit, 9);

            // init tip accounts
            jito::init_tip_accounts().await?;
            let amount_in_lamports = ui_amount_to_amount(*amount_in, 9);

            loop {
                let jupiter_swap_api_client = jupiter_swap_api_client.clone();
                let jupiter_extra_args = jupiter_extra_args.clone();
                let payer = payer.clone();
                let mint = *mint;
                let partner_fee = *partner_fee;
                let wait_for_confirmation = *wait_for_confirmation;
                tokio::spawn(async move {
                    run_arbitrage(
                        jupiter_swap_api_client,
                        jupiter_extra_args,
                        mint,
                        amount_in_lamports,
                        min_profit_lamports,
                        partner_fee,
                        &payer,
                        wait_for_confirmation,
                    )
                    .await
                });

                tokio::time::sleep(tokio::time::Duration::from_secs(*interval)).await;
            }
        }
    };
    Ok(())
}

pub async fn run_arbitrage(
    jupiter_swap_api_client: JupiterSwapApiClient,
    jupiter_extra_args: Option<HashMap<String, String>>,
    mint: Pubkey,
    amount_in_lamports: u64,
    min_profit_lamports: u64,
    partner_fee: f64,
    payer: &Keypair,
    wait_for_confirmation: bool,
) {
    let execution_id = uuid::Uuid::new_v4();

    let rpc_client = match get_rpc_client() {
        Ok(client) => client,
        Err(e) => {
            warn!("[{}] Failed to get RPC client: {}", execution_id, e);
            return;
        }
    };
    match arb::caculate_profit(
        &jupiter_swap_api_client,
        jupiter_extra_args.clone(),
        &amount_in_lamports,
        &spl_token::native_mint::id(),
        &mint,
        Dex::ALL,
        partner_fee,
    )
    .await
    {
        Ok((profit, quote_buy_response, quote_sell_response)) => {
            let profit_ui_amount = if profit < 0 {
                -1.0 * amount_to_ui_amount(profit.abs() as u64, 9)
            } else {
                amount_to_ui_amount(profit as u64, 9)
            };

            if profit < min_profit_lamports as i64 {
                debug!(
                    "[{}] ⏭️ Skip: {}, Profit: {} sol too small",
                    execution_id, mint, profit_ui_amount,
                );
            } else {
                info!(
                    "[{}] 💰 Found opportunity: {}, Profit: {} sol",
                    execution_id, mint, profit_ui_amount
                );
                match async {
                    let tip_lamports = profit as u64 / 2;
                    let tip_account = jito::get_tip_account().await?;
                    let tip_instruction =
                        tx::get_tip_instruction(&payer.pubkey(), &tip_account, tip_lamports);

                    let quote_response = arb::merge_quotes(
                        quote_buy_response,
                        quote_sell_response,
                        amount_in_lamports,
                        tip_lamports,
                    );

                    debug!(
                        "[{}] out_amount: {}, other_amount_threshold: {}",
                        execution_id,
                        quote_response.out_amount,
                        quote_response.other_amount_threshold
                    );

                    let mut tx_config = TransactionConfig::default();
                    tx_config.dynamic_compute_unit_limit = true;
                    tx_config.use_shared_accounts = Some(false);

                    let swap_instructions_response = arb::swap_instructions(
                        &jupiter_swap_api_client,
                        jupiter_extra_args,
                        &payer.pubkey(),
                        &quote_response,
                        tx_config,
                    )
                    .await?;

                    let mut ixs = arb::build_instructions(
                        swap_instructions_response.clone(),
                        tip_instruction,
                    );

                    // println!("ixs: {:#?}", ixs);
                    let versioned_transaction = create_tx_with_address_table_lookup(
                        &rpc_client,
                        &mut ixs,
                        &swap_instructions_response.address_lookup_table_addresses,
                        &payer,
                    )?;

                    tx::send_versioned_transaction(
                        &rpc_client,
                        &payer,
                        versioned_transaction,
                        wait_for_confirmation,
                    )
                    .await
                }
                .await
                {
                    Ok(_) => info!("[{}] 🚀 Arbitrage executed successfully", execution_id),
                    Err(e) => warn!("[{}] ⚠️ Failed to execute arbitrage: {}", execution_id, e),
                }
            }
        }
        Err(e) => {
            info!("Error calculating profit: {}", e);
        }
    }
}
