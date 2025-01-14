use std::env;

use anyhow::Result;
use clap::{Parser, Subcommand};
use jupiter_swap_api_client::transaction_config::ComputeUnitPriceMicroLamports;
use jupiter_swap_api_client::{
    JupiterSwapApiClient, quote::QuoteRequest, swap::SwapRequest,
    transaction_config::TransactionConfig,
};
use solana_arb::token::get_mint;
use solana_arb::{get_payer, get_rpc_client, logger};
use solana_sdk::pubkey::Pubkey;
use solana_sdk::signer::Signer;
use solana_sdk::transaction::VersionedTransaction;
use spl_token::ui_amount_to_amount;

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
        #[clap(help = "UI amount to swap in")]
        amount_in: f64,
        #[arg(long, help = "use jito to swap", default_value_t = false)]
        jito: bool,
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
    println!("Using base url: {}", api_base_url);
    let jupiter_swap_api_client = JupiterSwapApiClient::new(api_base_url);

    match &cli.command {
        Commands::Swap {
            mint,
            direction,
            amount_in,
            jito,
        } => {
            println!(
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
                dexes: Some("Whirlpool,Meteora DLMM,Raydium CLMM".into()),
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
    };

    // POST /swap-instructions
    // let swap_instructions = jupiter_swap_api_client
    //     .swap_instructions(&SwapRequest {
    //         user_public_key: TEST_WALLET,
    //         quote_response,
    //         config: TransactionConfig::default(),
    //     })
    //     .await
    //     .unwrap();
    // println!("swap_instructions: {swap_instructions:?}");

    Ok(())
}
