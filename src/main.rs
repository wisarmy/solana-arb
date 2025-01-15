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
use solana_arb::{arb, get_payer, get_rpc_client, jito, logger, tx};
use solana_sdk::pubkey::Pubkey;
use solana_sdk::signer::Signer;
use solana_sdk::transaction::VersionedTransaction;
use spl_token::{amount_to_ui_amount, ui_amount_to_amount};
use tracing::{info, warn};

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
        #[arg(long, help = "use jito to swap", default_value_t = false)]
        jito: bool,
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
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    dotenvy::dotenv().ok();
    let cli = Cli::parse();
    logger::init(true);

    let rpc_client = get_rpc_client()?;
    let payer = get_payer()?;

    let api_base_url = env::var("API_BASE_URL").unwrap_or("https://quote-api.jup.ag/v6".into());
    info!("Using jupiter api url: {}", api_base_url);
    let jupiter_swap_api_client = JupiterSwapApiClient::new(api_base_url);

    match &cli.command {
        Commands::Swap {
            mint,
            direction,
            amount_in,
            jito,
        } => {
            info!(
                "mint: {}, direction: {}, amount_in: {}, jito: {}",
                mint, direction, amount_in, jito
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
                ..QuoteRequest::default()
            };
            // GET /quote
            let quote_response = jupiter_swap_api_client.quote(&quote_request).await.unwrap();
            println!("{quote_response:#?}");
            let mut tx_config = TransactionConfig::default();
            tx_config.wrap_and_unwrap_sol = true;
            tx_config.compute_unit_price_micro_lamports =
                Some(ComputeUnitPriceMicroLamports::MicroLamports(50000));
            // POST /swap-instructions
            let swap_instructions = jupiter_swap_api_client
                .swap_instructions(&SwapRequest {
                    user_public_key: payer.pubkey(),
                    quote_response: quote_response.clone(),
                    config: tx_config,
                })
                .await
                .unwrap();
            println!("swap_instructions: {swap_instructions:#?}");
            // POST /swap
            let swap_response = jupiter_swap_api_client
                .swap(
                    &SwapRequest {
                        user_public_key: payer.pubkey(),
                        quote_response: quote_response.clone(),
                        config: TransactionConfig::default(),
                    },
                    None,
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
        } => {
            info!(
                "mint: {}, amount_in: {}, interval: {}s, min_profit: {} SOL",
                mint, amount_in, interval, min_profit
            );
            let min_profit_lamports = ui_amount_to_amount(*min_profit, 9);

            // init tip accounts
            jito::init_tip_accounts().await?;

            loop {
                let rpc_client = match get_rpc_client() {
                    Ok(client) => client,
                    Err(e) => {
                        warn!("Failed to get RPC client: {}", e);
                        continue;
                    }
                };
                match arb::caculate_profit(
                    &jupiter_swap_api_client,
                    &ui_amount_to_amount(*amount_in, 9),
                    &spl_token::native_mint::id(),
                    mint,
                    Dex::RAYDIUM | Dex::METEORA_DLMM | Dex::WHIRLPOOL,
                )
                .await
                {
                    Ok((profit, quote_buy_response, quote_sell_response)) => {
                        if profit < min_profit_lamports as i64 {
                            info!(
                                "Arb calculate {}, Profit: {} SOL too small, skip",
                                mint,
                                if profit < 0 {
                                    -1.0 * amount_to_ui_amount(profit.abs() as u64, 9)
                                } else {
                                    amount_to_ui_amount(profit as u64, 9)
                                },
                            );
                        } else {
                            info!(
                                "Arb calculate {}, Profit: {} sol",
                                mint,
                                amount_to_ui_amount(profit as u64, 9)
                            );

                            match async {
                                let buy_versioned_transaction = arb::swap(
                                    &jupiter_swap_api_client,
                                    &payer.pubkey(),
                                    &quote_buy_response,
                                )
                                .await?;
                                let sell_versioned_transaction = arb::swap(
                                    &jupiter_swap_api_client,
                                    &payer.pubkey(),
                                    &quote_sell_response,
                                )
                                .await?;
                                let versioned_transactions =
                                    vec![buy_versioned_transaction, sell_versioned_transaction];
                                let tip_lamports = profit as u64 / 2;
                                tx::send_transaction_with_tip(
                                    &rpc_client,
                                    &payer,
                                    versioned_transactions,
                                    tip_lamports,
                                    true,
                                )
                                .await
                            }
                            .await
                            {
                                Ok(_) => info!("Arbitrage executed successfully"),
                                Err(e) => info!("Failed to execute arbitrage: {}", e),
                            }
                        }
                    }
                    Err(e) => {
                        info!("Error calculating profit: {}", e);
                    }
                }

                tokio::time::sleep(tokio::time::Duration::from_secs(*interval)).await;
            }
        }
    };
    Ok(())
}
