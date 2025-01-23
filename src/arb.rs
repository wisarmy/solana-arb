use std::{collections::HashMap, env};

use anyhow::{Ok, Result, anyhow};
use jupiter_swap_api_client::{
    JupiterSwapApiClient,
    quote::{QuoteRequest, QuoteResponse},
    swap::{SwapInstructionsResponse, SwapRequest},
    transaction_config::TransactionConfig,
};
use rust_decimal::{Decimal, prelude::Zero};
use solana_sdk::{instruction::Instruction, pubkey::Pubkey, transaction::VersionedTransaction};
use tracing::{debug, trace, warn};

use crate::dex::Dex;

pub async fn caculate_profit(
    jupiter_swap_api_client: &JupiterSwapApiClient,
    jupiter_extra_args: Option<HashMap<String, String>>,
    amount_in: &u64,
    token_in: &Pubkey,
    token_out: &Pubkey,
    dexes: Dex,
    partner_fee: f64,
) -> Result<(i64, QuoteResponse, QuoteResponse)> {
    let slippage_bps = 0u16;
    let native_mint = spl_token::native_mint::id();
    if token_in != &native_mint {
        return Err(anyhow!("Only support swap from native mint"));
    }
    // decay factor
    let mut buy_decay_factor: f64 = env::var("BUY_DECAY_FACTOR")
        .unwrap_or("1.0".to_string())
        .parse()
        .unwrap_or(1.0);

    let mut sell_decay_factor: f64 = env::var("SELL_DECAY_FACTOR")
        .unwrap_or("1.0".to_string())
        .parse()
        .unwrap_or(1.0);

    if buy_decay_factor <= 0.0 || buy_decay_factor > 1.0 {
        warn!("Invalid buy_decay_factor: {}, using 1.0", buy_decay_factor);
        buy_decay_factor = 1.0;
    }

    if sell_decay_factor <= 0.0 || sell_decay_factor > 1.0 {
        warn!(
            "Invalid sell_decay_factor: {}, using 1.0",
            sell_decay_factor
        );
        sell_decay_factor = 1.0;
    }

    let quote_request = QuoteRequest {
        amount: *amount_in,
        input_mint: *token_in,
        output_mint: *token_out,
        dexes: Some(dexes.to_string()),
        slippage_bps,
        only_direct_routes: Some(true),
        quote_args: jupiter_extra_args.clone(),
        ..QuoteRequest::default()
    };
    let mut quote_buy_response = jupiter_swap_api_client.quote(&quote_request).await?;
    trace!("quote_buy_response: {:#?}", quote_buy_response);
    // buy decay factor
    let decayed_buy_out_amount = (quote_buy_response.out_amount as f64 * buy_decay_factor) as u64;
    let decayed_buy_other_amount_threshold =
        (quote_buy_response.other_amount_threshold as f64 * buy_decay_factor) as u64;
    debug!(
        "buy out amount: {}(decayed: {}), other amount threshold: {}(decayed: {})",
        quote_buy_response.out_amount,
        decayed_buy_out_amount,
        quote_buy_response.other_amount_threshold,
        decayed_buy_other_amount_threshold
    );
    quote_buy_response.out_amount = decayed_buy_out_amount;
    quote_buy_response.other_amount_threshold = decayed_buy_other_amount_threshold;

    let quote_request = QuoteRequest {
        amount: quote_buy_response.out_amount,
        input_mint: *token_out,
        output_mint: *token_in,
        dexes: Some(dexes.to_string()),
        slippage_bps,
        only_direct_routes: Some(true),
        quote_args: jupiter_extra_args,
        ..QuoteRequest::default()
    };

    let mut quote_sell_response = jupiter_swap_api_client.quote(&quote_request).await?;
    trace!("quote_sell_response: {:#?}", quote_sell_response);
    // sell decay factor
    let decayed_sell_out_amount =
        (quote_sell_response.out_amount as f64 * sell_decay_factor) as u64;
    let decayed_sell_other_amount_threshold =
        (quote_sell_response.other_amount_threshold as f64 * sell_decay_factor) as u64;
    debug!(
        "sell out amount: {}(decayed: {}), other amount threshold: {}(decayed: {})",
        quote_sell_response.out_amount,
        decayed_sell_out_amount,
        quote_sell_response.other_amount_threshold,
        decayed_sell_other_amount_threshold
    );
    quote_sell_response.out_amount = decayed_sell_out_amount;
    quote_sell_response.other_amount_threshold = decayed_sell_other_amount_threshold;

    let mut fee_amount = 0u64;
    quote_buy_response.route_plan.iter().for_each(|route| {
        if route.swap_info.fee_mint == native_mint {
            fee_amount += route.swap_info.fee_amount;
        }
    });
    debug!("swap fee amount (only caculate wsol): {}", fee_amount);
    let mut profit = quote_sell_response.out_amount as i64 - *amount_in as i64;
    profit = profit - fee_amount as i64;
    // caculate partner fee
    profit = profit - (*amount_in as f64 * partner_fee) as i64;

    Ok((profit, quote_buy_response, quote_sell_response))
}
// merge buy and sell quotes
pub fn merge_quotes(
    quote_buy_response: QuoteResponse,
    quote_sell_response: QuoteResponse,
    amount_in: u64,
    tip_lamports: u64,
) -> QuoteResponse {
    let mut merged_quote = quote_buy_response;

    // set output mint
    merged_quote.output_mint = quote_sell_response.output_mint;

    // set output amount
    merged_quote.out_amount = amount_in + tip_lamports;
    merged_quote.other_amount_threshold = amount_in + tip_lamports;

    // set price impact
    merged_quote.price_impact_pct = Decimal::zero();
    // merged_quote.price_impact_pct = Decimal::from_f64(1.0).unwrap();

    // set route plan
    let mut merged_route_plan = merged_quote.route_plan;
    merged_route_plan.extend(quote_sell_response.route_plan);
    merged_quote.route_plan = merged_route_plan;

    merged_quote
}

pub async fn swap(
    jupiter_swap_api_client: &JupiterSwapApiClient,
    jupiter_extra_args: Option<HashMap<String, String>>,
    user_public_key: &Pubkey,
    quote_response: &QuoteResponse,
) -> Result<VersionedTransaction> {
    let swap_response = jupiter_swap_api_client
        .swap(
            &SwapRequest {
                user_public_key: user_public_key.clone(),
                quote_response: quote_response.clone(),
                config: TransactionConfig {
                    use_shared_accounts: Some(false),
                    ..Default::default()
                },
            },
            jupiter_extra_args,
        )
        .await?;

    let versioned_transaction: VersionedTransaction =
        bincode::deserialize(&swap_response.swap_transaction).unwrap();
    Ok(versioned_transaction)
}

pub async fn swap_instructions(
    jupiter_swap_api_client: &JupiterSwapApiClient,
    jupiter_extra_args: Option<HashMap<String, String>>,
    user_public_key: &Pubkey,
    quote_response: &QuoteResponse,
) -> Result<SwapInstructionsResponse> {
    let swap_instructions = jupiter_swap_api_client
        .swap_instructions(
            &SwapRequest {
                user_public_key: user_public_key.clone(),
                quote_response: quote_response.clone(),
                config: TransactionConfig {
                    dynamic_compute_unit_limit: true,
                    use_shared_accounts: Some(false),
                    ..Default::default()
                },
            },
            jupiter_extra_args,
        )
        .await?;

    Ok(swap_instructions)
}

pub fn build_instructions(
    swap_instructions_response: SwapInstructionsResponse,
    tip_instruction: Instruction,
) -> Vec<Instruction> {
    let mut ixs = Vec::new();
    // compute budget instructions
    ixs.extend(swap_instructions_response.compute_budget_instructions);
    // token ledger instruction
    // if let Some(token_ledger) = swap_instructions_response.token_ledger_instruction {
    //     ixs.push(token_ledger);
    // }
    // setup
    ixs.extend(swap_instructions_response.setup_instructions);

    // swap
    ixs.push(swap_instructions_response.swap_instruction);
    // jito tips
    ixs.push(tip_instruction);
    // cleanup
    if let Some(cleanup) = swap_instructions_response.cleanup_instruction {
        ixs.push(cleanup);
    }
    // other instructions
    // ixs.extend(swap_instructions_response.other_instructions);

    ixs
}
