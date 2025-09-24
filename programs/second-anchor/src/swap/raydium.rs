use anchor_lang::solana_program::instruction::{AccountMeta, Instruction};
use crate::dex::raydium::RaydiumPoolState;
use crate::ComparePrices;
use anchor_lang::prelude::*;
use anchor_lang::solana_program::program::invoke;
use crate::optimalamt::OptimizationResult;

/// 执行Raydium AMM交易
pub fn execute_raydium_swap<'info>(
    pool_state: &RaydiumPoolState,
    token_program_index: usize,
    mint_token_account_index: usize,
    amount_in: u64,
    accounts: &[AccountInfo<'info>],
    ctx: &Context<ComparePrices<'info>>,
    is_buy: bool, // true: WSOL -> Token, false: Token -> WSOL
) -> Result<()> {


    let token_program = &accounts[token_program_index];
    let mint_token_account = &accounts[mint_token_account_index];
    
    // 获取池子相关账户
    let base_vault = &accounts[pool_state.base_vault_index];
    let quote_vault = &accounts[pool_state.quote_vault_index];
    

    let (input_token_account, output_token_account) = if is_buy {
        (
            ctx.accounts.wsol_token_account.to_account_info(),
            mint_token_account.to_account_info(),
        )
    } else {
        (
            mint_token_account.to_account_info(),
            ctx.accounts.wsol_token_account.to_account_info(),
        )
    };

 
    let pool = accounts[pool_state.pool_index].key();
    let mut account_metas = Vec::with_capacity(17);
     // 按照CPMM swap_base_input指令的账户顺序添加
     account_metas.push(AccountMeta::new_readonly(token_program.key(), false)); // Token Program
     account_metas.push(AccountMeta::new(pool, false)); //  Amm Id
     account_metas.push(AccountMeta::new_readonly(accounts[pool_state.event_authority_index].key(), false)); //  Amm Authority
     account_metas.push(AccountMeta::new(pool, false)); //   Amm Open Orders
     account_metas.push(AccountMeta::new(base_vault.key(), false)); //   Pool Coin Token Account
     account_metas.push(AccountMeta::new(quote_vault.key(), false)); //   Pool Pc Token Account
     account_metas.push(AccountMeta::new(pool, false)); //   Serum Program Id
     account_metas.push(AccountMeta::new(pool, false)); //   Serum Market
     account_metas.push(AccountMeta::new(pool, false)); //   Serum Bids
     account_metas.push(AccountMeta::new(pool, false)); //    Serum Asks
     account_metas.push(AccountMeta::new(pool, false)); //   - Serum Event Queue
     account_metas.push(AccountMeta::new(pool, false)); //   - Serum Coin Vault Account:
     account_metas.push(AccountMeta::new(pool, false)); //   - Serum Pc Vault Account:
     account_metas.push(AccountMeta::new(pool, false)); //   - Serum Vault Signer:
     account_metas.push(AccountMeta::new(input_token_account.key(), false)); //   - User Source Token Account
     account_metas.push(AccountMeta::new(output_token_account.key(), false)); //   - User Dest Token Account:
     account_metas.push(AccountMeta::new(ctx.accounts.payer.key(), true)); //   - User Owner
     

    // 🚀 优化：高效构建指令数据 (预分配容量)
    let mut instruction_data = Vec::with_capacity(17); // 1字节discriminator + 8字节amount_in + 8字节minimum_amount_out
    instruction_data.extend_from_slice(&[9]);  
    instruction_data.extend_from_slice(&amount_in.to_le_bytes());
    instruction_data.extend_from_slice(&0u64.to_le_bytes()); // minimum_amount_out
    

    // 构建指令
    let swap_instruction = Instruction {
        program_id: accounts[pool_state.program_id_index].key(),
        accounts: account_metas,
        data: instruction_data,
    };

    // 🚀 优化：构建账户信息数组 17个账户
    let account_infos = &[
        token_program.to_account_info(),
        accounts[pool_state.pool_index].to_account_info(),
        accounts[pool_state.event_authority_index].to_account_info(),
        accounts[pool_state.pool_index].to_account_info(),
        base_vault.to_account_info(),
        quote_vault.to_account_info(),
        accounts[pool_state.pool_index].to_account_info(),
        accounts[pool_state.pool_index].to_account_info(),
        accounts[pool_state.pool_index].to_account_info(),
        accounts[pool_state.pool_index].to_account_info(),
        accounts[pool_state.pool_index].to_account_info(),
        accounts[pool_state.pool_index].to_account_info(),
        accounts[pool_state.pool_index].to_account_info(),
        accounts[pool_state.pool_index].to_account_info(),
        input_token_account.to_account_info(),
        output_token_account.to_account_info(),
        ctx.accounts.payer.to_account_info(),
    ];

    // 执行原始指令调用 - 最高效的方式！
    invoke(&swap_instruction, account_infos).map_err(|e| e.into())
}

/// 执行Raydium AMM交易
pub fn execute_raydium_mid_swap<'info>(
    pool_state: &RaydiumPoolState,
    optimization_result: &OptimizationResult,
    amount_in: u64,
    accounts: &[AccountInfo<'info>],
    ctx: &Context<ComparePrices<'info>>,
) -> Result<()> {


    let token_program = ctx.accounts.token_program.to_account_info(); //raydium不支持2022
    // 获取池子相关账户
    let base_vault = &accounts[pool_state.base_vault_index];
    let quote_vault = &accounts[pool_state.quote_vault_index];
    let input_token_account = &accounts[optimization_result.token1_account_index];
    let output_token_account = &accounts[optimization_result.token2_account_index.unwrap()];

 
    let pool = accounts[pool_state.pool_index].key();
    let mut account_metas = Vec::with_capacity(17);

     account_metas.push(AccountMeta::new_readonly(token_program.key(), false)); // Token Program
     account_metas.push(AccountMeta::new(pool, false)); //  Amm Id
     account_metas.push(AccountMeta::new_readonly(accounts[pool_state.event_authority_index].key(), false)); //  Amm Authority
     account_metas.push(AccountMeta::new(pool, false)); //   Amm Open Orders
     account_metas.push(AccountMeta::new(base_vault.key(), false)); //   Pool Coin Token Account
     account_metas.push(AccountMeta::new(quote_vault.key(), false)); //   Pool Pc Token Account
     account_metas.push(AccountMeta::new(pool, false)); //   Serum Program Id
     account_metas.push(AccountMeta::new(pool, false)); //   Serum Market
     account_metas.push(AccountMeta::new(pool, false)); //   Serum Bids
     account_metas.push(AccountMeta::new(pool, false)); //    Serum Asks
     account_metas.push(AccountMeta::new(pool, false)); //   - Serum Event Queue
     account_metas.push(AccountMeta::new(pool, false)); //   - Serum Coin Vault Account:
     account_metas.push(AccountMeta::new(pool, false)); //   - Serum Pc Vault Account:
     account_metas.push(AccountMeta::new(pool, false)); //   - Serum Vault Signer:
     account_metas.push(AccountMeta::new(input_token_account.key(), false)); //   - User Source Token Account
     account_metas.push(AccountMeta::new(output_token_account.key(), false)); //   - User Dest Token Account:
     account_metas.push(AccountMeta::new(ctx.accounts.payer.key(), true)); //   - User Owner
     

    // 🚀 优化：高效构建指令数据 (预分配容量)
    let mut instruction_data = Vec::with_capacity(17); // 1字节discriminator + 8字节amount_in + 8字节minimum_amount_out
    instruction_data.extend_from_slice(&[9]);  
    instruction_data.extend_from_slice(&amount_in.to_le_bytes());
    instruction_data.extend_from_slice(&0u64.to_le_bytes()); // minimum_amount_out
    

    // 构建指令
    let swap_instruction = Instruction {
        program_id: accounts[pool_state.program_id_index].key(),
        accounts: account_metas,
        data: instruction_data,
    };

    // 🚀 优化：构建账户信息数组 17个账户
    let account_infos = &[
        token_program.to_account_info(),
        accounts[pool_state.pool_index].to_account_info(),
        accounts[pool_state.event_authority_index].to_account_info(),
        accounts[pool_state.pool_index].to_account_info(),
        base_vault.to_account_info(),
        quote_vault.to_account_info(),
        accounts[pool_state.pool_index].to_account_info(),
        accounts[pool_state.pool_index].to_account_info(),
        accounts[pool_state.pool_index].to_account_info(),
        accounts[pool_state.pool_index].to_account_info(),
        accounts[pool_state.pool_index].to_account_info(),
        accounts[pool_state.pool_index].to_account_info(),
        accounts[pool_state.pool_index].to_account_info(),
        accounts[pool_state.pool_index].to_account_info(),
        input_token_account.to_account_info(),
        output_token_account.to_account_info(),
        ctx.accounts.payer.to_account_info(),
    ];

    // 执行原始指令调用 - 最高效的方式！
    invoke(&swap_instruction, account_infos).map_err(|e| e.into())
}
