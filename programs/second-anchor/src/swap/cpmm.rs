use anchor_lang::prelude::*;
use anchor_lang::solana_program::instruction::{AccountMeta, Instruction};
use anchor_lang::solana_program::program::invoke;
use crate::dex::cpmm::CpmmPoolState;
use crate::ComparePrices;


/// 执行CPMM交易 (买入或卖出)
pub fn execute_cpmm_swap<'info>(
    pool_state: &CpmmPoolState,
    mint_token_index: usize,
    mint_program_index: usize,
    mint_token_account_index: usize,
    trade_amount: u64,
    accounts: &[AccountInfo<'info>],
    ctx: &Context<ComparePrices<'info>>,
    is_buy: bool, // true=买入(WSOL->Token), false=卖出(Token->WSOL)
) -> Result<()> {
   
    let wsol_mint = ctx.accounts.wsol_mint.key();

    // 根据买入/卖出确定输入输出金库
    let (input_vault, output_vault) = if is_buy {
        // 买入：WSOL -> Token
        if pool_state.token0_mint == wsol_mint {
            (
                accounts[pool_state.token0_vault_index].to_account_info(),
                accounts[pool_state.token1_vault_index].to_account_info(),
            )
        } else {
            (
                accounts[pool_state.token1_vault_index].to_account_info(),
                accounts[pool_state.token0_vault_index].to_account_info(),
            )
        }
    } else {
        // 卖出：Token -> WSOL
        if pool_state.token0_mint == wsol_mint {
            (
                accounts[pool_state.token1_vault_index].to_account_info(),
                accounts[pool_state.token0_vault_index].to_account_info(),
            )
        } else {
            (
                accounts[pool_state.token0_vault_index].to_account_info(),
                accounts[pool_state.token1_vault_index].to_account_info(),
            )
        }
    };

    // 根据买入/卖出确定输入输出账户和程序
    let (
        input_token_account,
        output_token_account,
        input_token_program,
        output_token_program,
        input_token_mint,
        output_token_mint,
    ) = if is_buy {
        // 买入：WSOL -> Token
        (
            ctx.accounts.wsol_token_account.to_account_info(),
            accounts[mint_token_account_index].to_account_info(),
            ctx.accounts.token_program.to_account_info(),
            accounts[mint_program_index].to_account_info(),
            ctx.accounts.wsol_mint.to_account_info(),
            accounts[mint_token_index].to_account_info(),
        )
    } else {
        // 卖出：Token -> WSOL
        (
            accounts[mint_token_account_index].to_account_info(),
            ctx.accounts.wsol_token_account.to_account_info(),
            accounts[mint_program_index].to_account_info(),
            ctx.accounts.token_program.to_account_info(),
            accounts[mint_token_index].to_account_info(),
            ctx.accounts.wsol_mint.to_account_info(),
        )
    };

    let mut account_metas = Vec::with_capacity(13);
    
    // 按照CPMM swap_base_input指令的账户顺序添加
    account_metas.push(AccountMeta::new_readonly(ctx.accounts.payer.key(), true)); // payer (signer)
    account_metas.push(AccountMeta::new_readonly(accounts[pool_state.authority_index].key(), false)); // authority
    account_metas.push(AccountMeta::new_readonly(accounts[pool_state.config_index].key(), false)); // amm_config
    account_metas.push(AccountMeta::new(accounts[pool_state.pool_index].key(), false)); // pool_state
    account_metas.push(AccountMeta::new(input_token_account.key(), false)); // input_token_account
    account_metas.push(AccountMeta::new(output_token_account.key(), false)); // output_token_account
    account_metas.push(AccountMeta::new(input_vault.key(), false)); // input_vault
    account_metas.push(AccountMeta::new(output_vault.key(), false)); // output_vault
    account_metas.push(AccountMeta::new_readonly(input_token_program.key(), false)); // input_token_program
    account_metas.push(AccountMeta::new_readonly(output_token_program.key(), false)); // output_token_program
    account_metas.push(AccountMeta::new_readonly(input_token_mint.key(), false)); // input_token_mint
    account_metas.push(AccountMeta::new_readonly(output_token_mint.key(), false)); // output_token_mint
    account_metas.push(AccountMeta::new(accounts[pool_state.observation_state_index].key(), false)); // observation_state

    // 🚀 优化：高效构建指令数据 (预分配容量)
    let mut instruction_data = Vec::with_capacity(24); // 8字节discriminator + 8字节amount_in + 8字节minimum_amount_out
    instruction_data.extend_from_slice(&[143, 190, 90, 218, 196, 30, 51, 222]); // swap_base_input指令discriminator
    instruction_data.extend_from_slice(&trade_amount.to_le_bytes());
    instruction_data.extend_from_slice(&0u64.to_le_bytes()); // minimum_amount_out

    // 构建指令
    let swap_instruction = Instruction {
        program_id: accounts[pool_state.program_id_index].key(),
        accounts: account_metas,
        data: instruction_data,
    };

    // 🚀 优化：构建账户信息数组
    let account_infos = &[
        ctx.accounts.payer.to_account_info(), // payer (signer)
        accounts[pool_state.authority_index].clone(), // authority
        accounts[pool_state.config_index].clone(), // amm_config
        accounts[pool_state.pool_index].clone(), // pool_state
        input_token_account.clone(), // input_token_account
        output_token_account.clone(), // output_token_account
        input_vault.clone(), // input_vault
        output_vault.clone(), // output_vault
        input_token_program.clone(), // input_token_program
        output_token_program.clone(), // output_token_program
        input_token_mint.clone(), // input_token_mint
        output_token_mint.clone(), // output_token_mint
        accounts[pool_state.observation_state_index].clone(), // observation_state
    ];

    // 执行原始指令调用 - 最高效的方式！
    invoke(&swap_instruction, account_infos).map_err(|e| e.into())
}
