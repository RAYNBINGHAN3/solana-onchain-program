use crate::dex::clmm::ClmmPoolState;
use crate::ComparePrices;
use anchor_lang::prelude::*;
use anchor_lang::solana_program::instruction::{AccountMeta, Instruction};
use anchor_lang::solana_program::program::invoke;


/// 执行CLMM交换
pub fn execute_clmm_swap<'info>(
    pool_state: &ClmmPoolState,
    token_mint_index: usize,
    mint_token_account_index: usize,
    amount: u64,
    accounts: &[AccountInfo<'info>],
    ctx: &Context<ComparePrices<'info>>,
    is_buy: bool, // true: WSOL->Token, false: Token->WSOL
) -> Result<()> {
    let wsol_mint = ctx.accounts.wsol_mint.key();
    // let token_mint = accounts[token_mint_index].key();
    // let program_id = accounts[token_program_index].key();
    let program_id = accounts[pool_state.program_id_index].key();

    // 确定交换方向和账户
    let (
        input_token_account,
        output_token_account,
        input_vault_index,
        output_vault_index,
        input_mint_info,
        output_mint_info,
    ) = if is_buy {
        // 买入：WSOL -> Token
        if pool_state.token_mint_0 == wsol_mint {
            (
                ctx.accounts.wsol_token_account.to_account_info(),
                accounts[mint_token_account_index].to_account_info(),
                pool_state.token_vault_0_index,
                pool_state.token_vault_1_index,
                ctx.accounts.wsol_mint.to_account_info(),
                accounts[token_mint_index].to_account_info(),
            )
        }else{
            (
                ctx.accounts.wsol_token_account.to_account_info(),
                accounts[mint_token_account_index].to_account_info(),
                pool_state.token_vault_1_index,
                pool_state.token_vault_0_index,
                ctx.accounts.wsol_mint.to_account_info(),
                accounts[token_mint_index].to_account_info(),
            )
        }
        
    } else {
        // 卖出：Token -> WSOL
        if pool_state.token_mint_0 == wsol_mint {
            (
                accounts[mint_token_account_index].to_account_info(),
                ctx.accounts.wsol_token_account.to_account_info(),
                pool_state.token_vault_1_index,
                pool_state.token_vault_0_index,
                accounts[token_mint_index].to_account_info(),
                ctx.accounts.wsol_mint.to_account_info(),
            )
        }else{
            (
                accounts[mint_token_account_index].to_account_info(),
                ctx.accounts.wsol_token_account.to_account_info(),
                pool_state.token_vault_0_index,
                pool_state.token_vault_1_index,
                accounts[token_mint_index].to_account_info(),
                ctx.accounts.wsol_mint.to_account_info(),
            )
        }
    };

    //过滤掉未初始化的tick array
    let tick_array_indices = [
        pool_state.tick_array_minus_1_index,
        pool_state.tick_array_0_index,
        pool_state.tick_array_1_index,
    ];
    let valid_tick_arrays = get_valid_tick_arrays(accounts, &tick_array_indices, program_id);

    // 构建CPI指令
    let swap_instruction = build_clmm_swap_instruction(
        program_id,
        pool_state,
        &ctx.accounts.payer.key(),
        input_token_account.key(),
        output_token_account.key(),
        input_vault_index,
        output_vault_index,
        input_mint_info.key(),
        output_mint_info.key(),
        amount,
        accounts,
        ctx,
        &valid_tick_arrays,
    )?;
    
    
    // 🚀 优化：预分配精确容量，避免扩容 (14个固定账户 + 最多3个tick_arrays)
    let mut account_infos = Vec::with_capacity(14 + valid_tick_arrays.len());
    
    // 按顺序添加固定账户
    account_infos.push(ctx.accounts.payer.to_account_info());
    account_infos.push(accounts[pool_state.amm_config_index].to_account_info());
    account_infos.push(accounts[pool_state.pool_index].to_account_info());
    account_infos.push(input_token_account);
    account_infos.push(output_token_account);
    account_infos.push(accounts[input_vault_index].to_account_info());
    account_infos.push(accounts[output_vault_index].to_account_info());
    account_infos.push(accounts[pool_state.observation_key_index].to_account_info());
    account_infos.push(ctx.accounts.token_program.to_account_info());
    account_infos.push(ctx.accounts.token_program_2022.to_account_info());
    account_infos.push(ctx.accounts.memo_program.to_account_info());
    account_infos.push(input_mint_info);
    account_infos.push(output_mint_info);
    account_infos.push(accounts[pool_state.bitmap_extension_index].to_account_info());

    // 添加有效的tick arrays
    account_infos.extend(valid_tick_arrays);

    invoke(&swap_instruction, &account_infos).map_err(|e| e.into())
}

/// 构建CLMM swap_v2指令
fn build_clmm_swap_instruction(
    program_id: Pubkey,
    pool_state: &ClmmPoolState,
    payer: &Pubkey,
    input_token_account: Pubkey,
    output_token_account: Pubkey,
    input_vault_index: usize,
    output_vault_index: usize,
    input_mint: Pubkey,
    output_mint: Pubkey,
    amount: u64,
    accounts: &[AccountInfo],
    ctx: &Context<ComparePrices>,
    valid_tick_arrays: &[AccountInfo],
) -> Result<Instruction> {
    // swap_v2指令的discriminator
    let discriminator: [u8; 8] = [43, 4, 237, 11, 26, 201, 30, 98];
     

    // 构建指令数据
    let mut instruction_data = Vec::with_capacity(41); // 8字节discriminator + 8字节amount + 8字节other_amount_threshold + 16字节sqrt_price_limit + 1字节is_base_input
    instruction_data.extend_from_slice(&discriminator);
    instruction_data.extend_from_slice(&amount.to_le_bytes()); // amount
    instruction_data.extend_from_slice(&0u64.to_le_bytes()); // other_amount_threshold
    instruction_data.extend_from_slice(&0u128.to_le_bytes()); // sqrt_price_limit (0 = no limit)
    instruction_data.push(1); // is_base_input = true

    // 🚀 优化：精确预分配容量 (14个固定账户 + 最多3个tick_arrays)
    let mut account_metas = Vec::with_capacity(14 + valid_tick_arrays.len());

    // 按照IDL顺序添加账户
    account_metas.push(AccountMeta::new(*payer, true)); // payer
    account_metas.push(AccountMeta::new_readonly(
        accounts[pool_state.amm_config_index].key(),
        false,
    )); // amm_config
    account_metas.push(AccountMeta::new(
        accounts[pool_state.pool_index].key(),
        false,
    )); // pool_state
    account_metas.push(AccountMeta::new(input_token_account, false)); // input_token_account
    account_metas.push(AccountMeta::new(output_token_account, false)); // output_token_account
    account_metas.push(AccountMeta::new(accounts[input_vault_index].key(), false)); // input_vault
    account_metas.push(AccountMeta::new(accounts[output_vault_index].key(), false)); // output_vault
    account_metas.push(AccountMeta::new(
        accounts[pool_state.observation_key_index].key(),
        false,
    )); // observation_state
    account_metas.push(AccountMeta::new_readonly(
        ctx.accounts.token_program.key(),
        false,
    )); // token_program
    account_metas.push(AccountMeta::new_readonly(
        ctx.accounts.token_program_2022.key(),
        false,
    )); // token_program_2022
    account_metas.push(AccountMeta::new_readonly(
        ctx.accounts.memo_program.key(),
        false,
    )); // memo_program
    account_metas.push(AccountMeta::new_readonly(input_mint, false)); // input_vault_mint
    account_metas.push(AccountMeta::new_readonly(output_mint, false)); // output_vault_mint
    // tick array + tick array bitmap extension
    account_metas.push(AccountMeta::new(
        accounts[pool_state.bitmap_extension_index].key(),
        false,
    )); // bitmap_extension
    
    // 🚀 优化：批量添加tick arrays，避免多次push
    for tick_array in valid_tick_arrays {
        account_metas.push(AccountMeta::new(tick_array.key(), false));
    }

    Ok(Instruction {
        program_id: program_id,
        accounts: account_metas,
        data: instruction_data,
    })
}



#[inline]
pub fn get_valid_tick_arrays<'info>(
    accounts: &[AccountInfo<'info>],
    tick_array_indices: &[usize],
    clmm_program_id: Pubkey,
) -> Vec<AccountInfo<'info>> {

    // 🚀 优化：预分配向量容量，减少内存重新分配
    let mut valid_arrays = Vec::with_capacity(tick_array_indices.len());

    for &index in tick_array_indices {
        let account = &accounts[index];
        
        if *account.owner == clmm_program_id {
            valid_arrays.push(account.to_account_info());
        }
    }

    valid_arrays
}