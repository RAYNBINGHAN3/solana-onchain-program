use anchor_lang::prelude::*;
use anchor_lang::solana_program::instruction::{AccountMeta, Instruction};
use anchor_lang::solana_program::program::invoke;
use crate::dex::dammv2::Dammv2PoolState;
use crate::ComparePrices;
 

pub fn execute_dammv2_swap<'info>(
    pool_state: &Dammv2PoolState,
    token_mint_index: usize,
    token_program_index: usize,
    token_account_index: usize,
    trade_amount: u64,
    accounts: &[AccountInfo<'info>],
    ctx: &Context<ComparePrices<'info>>,
    is_buy: bool, // true=买入(WSOL->Token), false=卖出(Token->WSOL)
) -> Result<()> {


    let wsol_mint = ctx.accounts.wsol_mint.key();

    // // 确定token a和b的mint和program (保持不变，因为DAMM_V2的a/b是固定的)
    let (token_a_mint, token_b_mint, token_a_program, token_b_program) =
        if pool_state.token_a_mint == wsol_mint {
            (
                ctx.accounts.wsol_mint.to_account_info(),
                accounts[token_mint_index].to_account_info(),
                ctx.accounts.token_program.to_account_info(),
                accounts[token_program_index].to_account_info(),
            )
        } else {
            (
                accounts[token_mint_index].to_account_info(),
                ctx.accounts.wsol_mint.to_account_info(),
                accounts[token_program_index].to_account_info(),
                ctx.accounts.token_program.to_account_info(),
            )
        };

    // // 根据买入/卖出确定输入输出账户
    let (user_token_in, user_token_out, referral_token_account) = if is_buy {
        // 买入：WSOL -> Token
        (
            ctx.accounts.wsol_token_account.to_account_info(),
            accounts[token_account_index].to_account_info(),
            accounts[pool_state.program_id_index].to_account_info(), // 默认都是wsol作为手续费 懒得去判断
        )
    } else {
        // 卖出：Token -> WSOL
        (
            accounts[token_account_index].to_account_info(),
            ctx.accounts.wsol_token_account.to_account_info(),
            accounts[pool_state.program_id_index].to_account_info(),
        )
    };



    // 构建账户列表
    let account_metas: [AccountMeta; 14] = [
        AccountMeta::new_readonly(accounts[pool_state.pool_authority_index].key(), false),
        AccountMeta::new(accounts[pool_state.pool_index].key(), false),
        AccountMeta::new(user_token_in.key(), false),
        AccountMeta::new(user_token_out.key(), false),
        AccountMeta::new(accounts[pool_state.token_a_vault_index].key(), false),
        AccountMeta::new(accounts[pool_state.token_b_vault_index].key(), false),
        AccountMeta::new_readonly(token_a_mint.key(), false),
        AccountMeta::new_readonly(token_b_mint.key(), false),
        AccountMeta::new_readonly(ctx.accounts.payer.key(), true),
        AccountMeta::new_readonly(token_a_program.key(), false),
        AccountMeta::new_readonly(token_b_program.key(), false),
        AccountMeta::new_readonly(referral_token_account.key(), false), //referral_token_account
        AccountMeta::new_readonly(accounts[pool_state.event_authority_index].key(), false),
        AccountMeta::new_readonly(accounts[pool_state.program_id_index].key(), false),
    ];


    // 🚀 优化: 高效构建指令数据 (预分配容量，避免序列化)
    let mut instruction_data = Vec::with_capacity(24); // 8字节discriminator + 8字节amount_in + 8字节minimum_amount_out
    instruction_data.extend_from_slice(&[248, 198, 158, 145, 225, 117, 135, 200]); // swap指令discriminator
    instruction_data.extend_from_slice(&trade_amount.to_le_bytes());
    instruction_data.extend_from_slice(&0u64.to_le_bytes()); // minimum_amount_out


    // 构建指令
    let swap_instruction = Instruction {
        program_id: accounts[pool_state.program_id_index].key(),
        accounts: account_metas.into(),
        data: instruction_data,
    };

    // 🚀 优化: 直接构建切片，避免Vec的堆分配
    let account_infos = &[
        accounts[pool_state.pool_authority_index].clone(),
        accounts[pool_state.pool_index].clone(),
        user_token_in.clone(),
        user_token_out.clone(),
        accounts[pool_state.token_a_vault_index].clone(),
        accounts[pool_state.token_b_vault_index].clone(),
        token_a_mint.clone(),
        token_b_mint.clone(),
        ctx.accounts.payer.to_account_info(),
        token_a_program.clone(),
        token_b_program.clone(),
        accounts[pool_state.program_id_index].clone(),
        accounts[pool_state.event_authority_index].clone(),
        accounts[pool_state.program_id_index].clone(),
    ];

    // 执行CPI调用
    invoke(&swap_instruction, account_infos).map_err(|e| e.into())
}

