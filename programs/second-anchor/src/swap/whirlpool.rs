use crate::dex::whirlpool::WhirlpoolPoolState;
use crate::ComparePrices;
use anchor_lang::prelude::*;
use anchor_lang::solana_program::instruction::{AccountMeta, Instruction};
use anchor_lang::solana_program::program::invoke;

/// 执行 Whirlpool 交换
pub fn execute_whirlpool_swap<'info>(
    pool_state: &WhirlpoolPoolState,
    token_mint_program_index: usize,
    token_mint_index: usize,
    mint_token_account_index: usize,
    amount: u64,
    accounts: &[AccountInfo<'info>],
    ctx: &Context<ComparePrices<'info>>,
    is_buy: bool, // true: WSOL->Token, false: Token->WSOL
) -> Result<()> {
    let wsol_mint = ctx.accounts.wsol_mint.key();
    let program_id = accounts[pool_state.program_id_index].key();

    // 确定交换方向和账户
    let (
        input_token_program,
        output_token_program,
        input_token_account,
        output_token_account,
        input_vault_index,
        output_vault_index,
        input_mint_info,
        output_mint_info,
        a_to_b,
    ) = if is_buy {
        // 买入：WSOL -> Token
        if pool_state.token_mint_a == wsol_mint {
            (
                ctx.accounts.token_program.to_account_info(),
                accounts[token_mint_program_index].to_account_info(),
                ctx.accounts.wsol_token_account.to_account_info(),
                accounts[mint_token_account_index].to_account_info(),
                pool_state.vault_a_index,
                pool_state.vault_b_index,
                ctx.accounts.wsol_mint.to_account_info(),
                accounts[token_mint_index].to_account_info(),
                true, // a_to_b = true (WSOL is token A)
            )
        } else {
            (
                ctx.accounts.token_program.to_account_info(),
                accounts[token_mint_program_index].to_account_info(),
                ctx.accounts.wsol_token_account.to_account_info(),
                accounts[mint_token_account_index].to_account_info(),
                pool_state.vault_b_index,
                pool_state.vault_a_index,
                ctx.accounts.wsol_mint.to_account_info(),
                accounts[token_mint_index].to_account_info(),
                false, // a_to_b = false (WSOL is token B)
            )
        }
    } else {
        // 卖出：Token -> WSOL
        if pool_state.token_mint_a == wsol_mint {
            (
                accounts[token_mint_program_index].to_account_info(),
                ctx.accounts.token_program.to_account_info(),
                accounts[mint_token_account_index].to_account_info(),
                ctx.accounts.wsol_token_account.to_account_info(),
                pool_state.vault_b_index,
                pool_state.vault_a_index,
                accounts[token_mint_index].to_account_info(),
                ctx.accounts.wsol_mint.to_account_info(),
                false, // a_to_b = false (Token B -> WSOL A)
            )
        } else {
            (
                accounts[token_mint_program_index].to_account_info(),
                ctx.accounts.token_program.to_account_info(),
                accounts[mint_token_account_index].to_account_info(),
                ctx.accounts.wsol_token_account.to_account_info(),
                pool_state.vault_a_index,
                pool_state.vault_b_index,
                accounts[token_mint_index].to_account_info(),
                ctx.accounts.wsol_mint.to_account_info(),
                true, // a_to_b = true (Token A -> WSOL B)
            )
        }
    };

  
   
    // 构建 CPI 指令
    let swap_instruction = build_whirlpool_swap_instruction(
        program_id,
        pool_state,
        &ctx.accounts.payer.key(),
        input_token_program.key(),
        output_token_program.key(),
        input_token_account.key(),
        output_token_account.key(),
        input_vault_index,
        output_vault_index,
        input_mint_info.key(),
        output_mint_info.key(),
        amount,
        a_to_b,
        accounts,
        ctx
    )?;

    // 构建账户信息数组
    let account_infos = vec![
        input_token_program,       // tokenProgramA
        output_token_program,  // tokenProgramB
        ctx.accounts.memo_program.to_account_info(),        // memoProgram
        ctx.accounts.payer.to_account_info(),               // tokenAuthority
        accounts[pool_state.pool_index].to_account_info(),  // whirlpool
        input_mint_info,                                    // tokenMintA/B
        output_mint_info,                                   // tokenMintB/A
        input_token_account,                                // tokenOwnerAccountA/B
        accounts[input_vault_index].to_account_info(),      // tokenVaultA/B
        output_token_account,                               // tokenOwnerAccountB/A
        accounts[output_vault_index].to_account_info(),     // tokenVaultB/A
        accounts[pool_state.tick_array_0_index].to_account_info(), // tickArray0
        accounts[pool_state.tick_array_1_index].to_account_info(), // tickArray1
        accounts[pool_state.tick_array_2_index].to_account_info(), // tickArray2
        accounts[pool_state.oracle_index].to_account_info(), // oracle
    ];

    // 执行 CPI 调用
    invoke(&swap_instruction, &account_infos).map_err(|e| e.into())
}

/// 构建 Whirlpool swapV2 指令
fn build_whirlpool_swap_instruction(
    program_id: Pubkey,
    pool_state: &WhirlpoolPoolState,
    payer: &Pubkey,
    input_token_program: Pubkey,
    output_token_program: Pubkey,
    input_token_account: Pubkey,
    output_token_account: Pubkey,
    input_vault_index: usize,
    output_vault_index: usize,
    input_mint: Pubkey,
    output_mint: Pubkey,
    amount: u64,
    a_to_b: bool,
    accounts: &[AccountInfo],
    ctx: &Context<ComparePrices>,
) -> Result<Instruction> {
    // swapV2 指令的 discriminator
    let discriminator: [u8; 8] = [43, 4, 237, 11, 26, 201, 30, 98];

    // 构建指令数据
    let mut instruction_data = Vec::with_capacity(42); // 8字节discriminator + 8字节amount + 8字节otherAmountThreshold + 16字节sqrtPriceLimit + 1字节amountSpecifiedIsInput + 1字节aToB + 可选的remainingAccountsInfo
    instruction_data.extend_from_slice(&discriminator);
    instruction_data.extend_from_slice(&amount.to_le_bytes());          // amount
    instruction_data.extend_from_slice(&0u64.to_le_bytes());            // otherAmountThreshold
    instruction_data.extend_from_slice(&0u128.to_le_bytes());           // sqrtPriceLimit (0 = no limit)
    instruction_data.push(1);                                           // amountSpecifiedIsInput = true
    instruction_data.push(if a_to_b { 1 } else { 0 });                // aToB
    instruction_data.push(0);                                           // remainingAccountsInfo = None

    // 构建账户列表
    let mut account_metas = Vec::with_capacity(15);

    // 按照 IDL 顺序添加账户
    account_metas.push(AccountMeta::new_readonly(
        input_token_program,
        false,
    )); // tokenProgramA
    account_metas.push(AccountMeta::new_readonly(
        output_token_program,
        false,
    )); // tokenProgramB

    account_metas.push(AccountMeta::new_readonly(
        ctx.accounts.memo_program.key(),
        false,
    )); // memoProgram

    account_metas.push(AccountMeta::new(*payer, true)); // tokenAuthority
    account_metas.push(AccountMeta::new(
        accounts[pool_state.pool_index].key(),
        false,
    )); // whirlpool
    
    account_metas.push(AccountMeta::new_readonly(input_mint, false)); // tokenMintA or tokenMintB
    account_metas.push(AccountMeta::new_readonly(output_mint, false)); // tokenMintB or tokenMintA
    
    account_metas.push(AccountMeta::new(input_token_account, false)); // tokenOwnerAccountA or tokenOwnerAccountB
    account_metas.push(AccountMeta::new(
        accounts[input_vault_index].key(),
        false,
    )); // tokenVaultA or tokenVaultB
    account_metas.push(AccountMeta::new(output_token_account, false)); // tokenOwnerAccountB or tokenOwnerAccountA
    account_metas.push(AccountMeta::new(
        accounts[output_vault_index].key(),
        false,
    )); // tokenVaultB or tokenVaultA

    // 添加 tick arrays
    account_metas.push(AccountMeta::new(
        accounts[pool_state.tick_array_0_index].key(),
        false,
    )); // tickArray0
    account_metas.push(AccountMeta::new(
        accounts[pool_state.tick_array_1_index].key(),
        false,
    )); // tickArray1
    account_metas.push(AccountMeta::new(
        accounts[pool_state.tick_array_2_index].key(),
        false,
    )); // tickArray2

    // 添加 oracle
    account_metas.push(AccountMeta::new(
        accounts[pool_state.oracle_index].key(),
        false,
    )); // oracle

    Ok(Instruction {
        program_id,
        accounts: account_metas,
        data: instruction_data,
    })
}

 
