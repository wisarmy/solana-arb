use anyhow::{Ok, Result, anyhow};
use jupiter_swap_api_client::{
    JupiterSwapApiClient,
    quote::{QuoteRequest, QuoteResponse},
    swap::SwapRequest,
    transaction_config::TransactionConfig,
};
use solana_sdk::{pubkey::Pubkey, transaction::VersionedTransaction};
use tracing::{debug, trace};

use crate::dex::Dex;

pub async fn caculate_profit(
    jupiter_swap_api_client: &JupiterSwapApiClient,
    amount_in: &u64,
    token_in: &Pubkey,
    token_out: &Pubkey,
    dexes: Dex,
) -> Result<(i64, QuoteResponse, QuoteResponse)> {
    let native_mint = spl_token::native_mint::id();
    if token_in != &native_mint {
        return Err(anyhow!("Only support swap from native mint"));
    }

    let quote_request = QuoteRequest {
        amount: *amount_in,
        input_mint: *token_in,
        output_mint: *token_out,
        dexes: Some(dexes.to_string()),
        slippage_bps: 0,
        ..QuoteRequest::default()
    };
    let quote_buy_response = jupiter_swap_api_client.quote(&quote_request).await?;
    trace!("quote_buy_response: {:#?}", quote_buy_response);

    let quote_request = QuoteRequest {
        amount: quote_buy_response.other_amount_threshold,
        input_mint: *token_out,
        output_mint: *token_in,
        dexes: Some(dexes.to_string()),
        slippage_bps: 0,
        ..QuoteRequest::default()
    };
    let quote_sell_response = jupiter_swap_api_client.quote(&quote_request).await?;
    trace!("quote_sell_response: {:#?}", quote_sell_response);

    let mut fee_amount = 0u64;
    quote_buy_response.route_plan.iter().for_each(|route| {
        if route.swap_info.fee_mint == native_mint {
            fee_amount += route.swap_info.fee_amount;
        }
    });
    debug!("swap fee amount (only caculate wsol): {}", fee_amount);
    let mut profit = quote_sell_response.other_amount_threshold as i64 - *amount_in as i64;
    profit = profit - fee_amount as i64;
    Ok((profit, quote_buy_response, quote_sell_response))
}

pub async fn swap(
    jupiter_swap_api_client: &JupiterSwapApiClient,
    user_public_key: &Pubkey,
    quote_response: &QuoteResponse,
) -> Result<VersionedTransaction> {
    let swap_response = jupiter_swap_api_client
        .swap(
            &SwapRequest {
                user_public_key: user_public_key.clone(),
                quote_response: quote_response.clone(),
                config: TransactionConfig::default(),
            },
            None,
        )
        .await?;

    let versioned_transaction: VersionedTransaction =
        bincode::deserialize(&swap_response.swap_transaction).unwrap();
    Ok(versioned_transaction)
}
