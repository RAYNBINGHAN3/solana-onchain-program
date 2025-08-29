use anchor_lang::prelude::*;
use anchor_lang::solana_program::instruction::{AccountMeta, Instruction};
use anchor_lang::solana_program::program::invoke;
use crate::dex::dlmm::DlmmPoolState;
use crate::ComparePrices;
 

/// 执行DLMM交易 (买入或卖出)
pub fn execute_dlmm_swap<'info>(
    pool_state: &DlmmPoolState,
    token_mint_index: usize,
    token_program_index: usize,
    token_account_index: usize,
    bin_array_minus_1_index: usize,
    bin_array_0_index: usize,
    bin_array_1_index: usize,
    reserve_x_index: usize,
    reserve_y_index: usize,
    trade_amount: u64,
    accounts: &[AccountInfo<'info>],
    ctx: &Context<ComparePrices<'info>>,
    is_buy: bool, // true=买入(WSOL->Token), false=卖出(Token->WSOL)
) -> Result<()> {


    let wsol_mint = ctx.accounts.wsol_mint.key();

    // 确定token X和Y的mint和program (保持不变，因为DLMM的X/Y是固定的)
    let (token_x_mint, token_y_mint, token_x_program, token_y_program) =
        if pool_state.token_x_mint == wsol_mint {
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

    // 根据买入/卖出确定输入输出账户
    let (user_token_in, user_token_out) = if is_buy {
        // 买入：WSOL -> Token
        (
            ctx.accounts.wsol_token_account.to_account_info(),
            accounts[token_account_index].to_account_info(),
        )
    } else {
        // 卖出：Token -> WSOL
        (
            accounts[token_account_index].to_account_info(),
            ctx.accounts.wsol_token_account.to_account_info(),
        )
    };

 
    let bin_array_indices = [bin_array_minus_1_index, bin_array_0_index, bin_array_1_index];
    let valid_bin_arrays = get_valid_bin_arrays(accounts, &bin_array_indices, pool_state.program_id_index);


    // 构建固定账户的AccountMeta (13个固定账户 + bin arrays)
    let mut account_metas = Vec::with_capacity(13 + valid_bin_arrays.len());
    
    // 按照DLMM swap指令的账户顺序添加
    account_metas.push(AccountMeta::new(accounts[pool_state.pool_index].key(), false)); // lb_pair
    account_metas.push(AccountMeta::new_readonly(accounts[pool_state.program_id_index].key(), false)); // bin_array_bitmap_extension
    account_metas.push(AccountMeta::new(accounts[reserve_x_index].key(), false)); // reserve_x
    account_metas.push(AccountMeta::new(accounts[reserve_y_index].key(), false)); // reserve_y
    account_metas.push(AccountMeta::new(user_token_in.key(), false)); // user_token_in
    account_metas.push(AccountMeta::new(user_token_out.key(), false)); // user_token_out
    account_metas.push(AccountMeta::new_readonly(token_x_mint.key(), false)); // token_x_mint
    account_metas.push(AccountMeta::new_readonly(token_y_mint.key(), false)); // token_y_mint
    account_metas.push(AccountMeta::new(accounts[pool_state.oracle_index].key(), false)); // oracle
    account_metas.push(AccountMeta::new_readonly(accounts[pool_state.program_id_index].key(), false)); // host_fee_in
    account_metas.push(AccountMeta::new_readonly(ctx.accounts.payer.key(), true)); // user (signer)
    account_metas.push(AccountMeta::new_readonly(token_x_program.key(), false)); // token_x_program
    account_metas.push(AccountMeta::new_readonly(token_y_program.key(), false)); // token_y_program
    account_metas.push(AccountMeta::new_readonly(accounts[pool_state.event_authority_index].key(), false)); // event_authority
    account_metas.push(AccountMeta::new_readonly(accounts[pool_state.program_id_index].key(), false)); // program
    
    // 添加有效的bin arrays作为remaining accounts
    for bin_array in &valid_bin_arrays {
        account_metas.push(AccountMeta::new(bin_array.key(), false));
    }

    // 🚀 优化：高效构建指令数据 (预分配容量)
    let mut instruction_data = Vec::with_capacity(24); // 8字节discriminator + 8字节amount_in + 8字节minimum_amount_out
    instruction_data.extend_from_slice(&[248, 198, 158, 145, 225, 117, 135, 200]); // swap指令discriminator
    instruction_data.extend_from_slice(&trade_amount.to_le_bytes());
    instruction_data.extend_from_slice(&0u64.to_le_bytes()); // minimum_amount_out

    // 构建指令
    let swap_instruction = Instruction {
        program_id: accounts[pool_state.program_id_index].key(),
        accounts: account_metas,
        data: instruction_data,
    };

    // 🚀 优化：构建账户信息数组
    let mut account_infos = Vec::with_capacity(13 + valid_bin_arrays.len());
    account_infos.push(accounts[pool_state.pool_index].clone()); // lb_pair
    // 跳过 bin_array_bitmap_extension (可选账户)
    account_infos.push(accounts[reserve_x_index].clone()); // reserve_x
    account_infos.push(accounts[reserve_y_index].clone()); // reserve_y
    account_infos.push(user_token_in.clone()); // user_token_in
    account_infos.push(user_token_out.clone()); // user_token_out
    account_infos.push(token_x_mint.clone()); // token_x_mint
    account_infos.push(token_y_mint.clone()); // token_y_mint
    account_infos.push(accounts[pool_state.oracle_index].clone()); // oracle
    // 跳过 host_fee_in (可选账户)
    account_infos.push(ctx.accounts.payer.to_account_info()); // user (signer)
    account_infos.push(token_x_program.clone()); // token_x_program
    account_infos.push(token_y_program.clone()); // token_y_program
    account_infos.push(accounts[pool_state.event_authority_index].clone()); // event_authority
    account_infos.push(accounts[pool_state.program_id_index].clone()); // program
    
    // 添加有效的bin arrays
    account_infos.extend(valid_bin_arrays);

    // 执行原始指令调用 - 最高效的方式！
    invoke(&swap_instruction, &account_infos).map_err(|e| e.into())
}

/// 🚀 高效过滤有效的bin arrays
/// 优化：预分配容量，直接比较Pubkey，最小化内存分配
#[inline]
pub fn get_valid_bin_arrays<'info>(
    accounts: &[AccountInfo<'info>],
    bin_array_indices: &[usize],
    program_id_index: usize,
) -> Vec<AccountInfo<'info>> {
    // 🚀 优化：一次性解析程序ID，避免重复字符串操作
    let dlmm_program_id = accounts[program_id_index].key();
    
    // 🚀 优化：预分配向量容量，减少内存重新分配
    let mut valid_arrays = Vec::with_capacity(bin_array_indices.len());
    
    for &index in bin_array_indices {
        let account = &accounts[index];
        // 🚀 优化：直接比较Pubkey，比字符串比较更高效
        if *account.owner == dlmm_program_id {
            valid_arrays.push(account.to_account_info());
        }
    }
    
    valid_arrays
}
