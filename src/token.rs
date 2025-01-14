use anyhow::Result;
use solana_client::rpc_client::RpcClient;
use solana_sdk::{program_pack::Pack, pubkey::Pubkey};
use spl_token::state::Mint;

pub fn get_mint(rpc_client: &RpcClient, address: &Pubkey) -> Result<Mint> {
    let mint_account = rpc_client.get_account(address)?;
    let mint_data = Mint::unpack(&mint_account.data)?;
    Ok(mint_data)
}
