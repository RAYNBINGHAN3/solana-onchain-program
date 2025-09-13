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
        token_program_a,
        token_program_b,
        token_account_a,
        token_account_b,
        mint_a,
        mint_b,
        a_to_b,
    ) = if is_buy {
        // 买入：WSOL -> Token
        if pool_state.token_mint_a == wsol_mint {
            (
                ctx.accounts.token_program.to_account_info(),
                accounts[token_mint_program_index].to_account_info(),
                ctx.accounts.wsol_token_account.to_account_info(),
                accounts[mint_token_account_index].to_account_info(),
      
                ctx.accounts.wsol_mint.to_account_info(),
                accounts[token_mint_index].to_account_info(),
                true, // a_to_b = true (WSOL is token A)
            )
        } else {
            (
                accounts[token_mint_program_index].to_account_info(),
                ctx.accounts.token_program.to_account_info(),
                accounts[mint_token_account_index].to_account_info(),
                ctx.accounts.wsol_token_account.to_account_info(),
        
                accounts[token_mint_index].to_account_info(),
                ctx.accounts.wsol_mint.to_account_info(),
                false, // a_to_b = false (WSOL is token B)
            )
        }
    } else {
        // 卖出：Token -> WSOL
        if pool_state.token_mint_a == wsol_mint {
            (
                ctx.accounts.token_program.to_account_info(),
                accounts[token_mint_program_index].to_account_info(),
                ctx.accounts.wsol_token_account.to_account_info(),
                accounts[mint_token_account_index].to_account_info(),
       
                ctx.accounts.wsol_mint.to_account_info(),
                accounts[token_mint_index].to_account_info(),
                false, // a_to_b = false (Token B -> WSOL A)
            )
        } else {
            (
                accounts[token_mint_program_index].to_account_info(),
                ctx.accounts.token_program.to_account_info(),
                accounts[mint_token_account_index].to_account_info(),
                ctx.accounts.wsol_token_account.to_account_info(),
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
        token_program_a.key(),
        token_program_b.key(),
        token_account_a.key(),
        token_account_b.key(),
        pool_state.vault_a_index,
        pool_state.vault_b_index,
        mint_a.key(),
        mint_b.key(),
        amount,
        a_to_b,
        accounts,
        ctx
    )?;

    // 构建账户信息数组
    let account_infos = vec![
        token_program_a,       // tokenProgramA
        token_program_b,  // tokenProgramB
        ctx.accounts.memo_program.to_account_info(),        // memoProgram
        ctx.accounts.payer.to_account_info(),               // tokenAuthority
        accounts[pool_state.pool_index].to_account_info(),  // whirlpool
        mint_a,                                    // tokenMintA
        mint_b,                                   // tokenMintB
        token_account_a,                                // tokenOwnerAccountA
        accounts[pool_state.vault_a_index].to_account_info(),      // tokenVaultA
        token_account_b,                               // tokenOwnerAccountB
        accounts[pool_state.vault_b_index].to_account_info(),     // tokenVaultB
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
    token_program_a: Pubkey,
    token_program_b: Pubkey,
    token_account_a: Pubkey,
    token_account_b: Pubkey,
    vault_index_a: usize,
    vault_index_b: usize,
    mint_a: Pubkey,
    mint_b: Pubkey,
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
        token_program_a,
        false,
    )); // tokenProgramA
    account_metas.push(AccountMeta::new_readonly(
        token_program_b,
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
    
    account_metas.push(AccountMeta::new_readonly(mint_a, false)); // tokenMintA 
    account_metas.push(AccountMeta::new_readonly(mint_b, false)); // tokenMintB 
    
    account_metas.push(AccountMeta::new(token_account_a, false)); // tokenOwnerAccountA 
    account_metas.push(AccountMeta::new(
        accounts[vault_index_a].key(),
        false,
    )); // tokenVaultA 
    account_metas.push(AccountMeta::new(token_account_b, false)); // tokenOwnerAccountB 
    account_metas.push(AccountMeta::new(
        accounts[vault_index_b].key(),
        false,
    )); // tokenVaultB 

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

 
