use anchor_lang::prelude::*;
use anchor_lang::solana_program::instruction::{AccountMeta, Instruction};
use anchor_lang::solana_program::program::invoke;
use crate::dex::pump::PumpPoolState;
use crate::ComparePrices;
use crate::dex::pump::pump_buy_base_input_internal;
use crate::constant::WSOL_MINT;
use crate::optimalamt::OptimizationResult;

/// Pump AMM 指令 discriminators
const BUY_DISCRIMINATOR: [u8; 8] = [102, 6, 61, 18, 1, 218, 235, 234];
const SELL_DISCRIMINATOR: [u8; 8] = [51, 230, 133, 164, 1, 127, 131, 173];

/// 执行 Pump AMM 交易 (买入或卖出)
pub fn execute_pump_swap<'info>(
    pool_state: &PumpPoolState,
    token_mint_index: usize,
    token_program_index: usize,
    token_account_index: usize,
    trade_amount: u64,
    max_or_min_amount_out: u64,
    accounts: &[AccountInfo<'info>],
    ctx: &Context<ComparePrices<'info>>,
    is_buy: bool, // true=买入(WSOL->Token), false=卖出(Token->WSOL)
) -> Result<()> {
    // let wsol_mint = ctx.accounts.wsol_mint.key();
    let mut max_or_min_amount_out = max_or_min_amount_out;
    if is_buy {
        if WSOL_MINT == pool_state.quote_mint {
            execute_pump_buy(
                pool_state,
                token_mint_index,
                token_program_index,
                token_account_index,
                trade_amount,
                &mut max_or_min_amount_out,
                accounts,
                ctx,
            )
        } else {
            execute_pump_sell(
                pool_state,
                token_mint_index,
                token_program_index,
                token_account_index,
                trade_amount,
                0,
                accounts,
                ctx,
            )
        }

    } else {
        if WSOL_MINT == pool_state.quote_mint {
            execute_pump_sell(
                pool_state,
                token_mint_index,
                token_program_index,
                token_account_index,
                trade_amount,
                0,
                accounts,
                ctx,
            )
        } else {
          
            execute_pump_buy(
                pool_state,
                token_mint_index,
                token_program_index,
                token_account_index,
                trade_amount, // token in amount
                &mut max_or_min_amount_out, //此时max_or_min_amount_out为wsol wsol_amount为token
                accounts,
                ctx,
            )
        }    
    }
}

/// 执行 Pump 买入交易 (WSOL -> Token)
fn execute_pump_buy<'info>(
    pool_state: &PumpPoolState,
    token_mint_index: usize,
    token_program_index: usize,
    token_account_index: usize,
    wsol_amount: u64,
    max_token_amount_out: &mut u64,
    accounts: &[AccountInfo<'info>],
    ctx: &Context<ComparePrices<'info>>,
) -> Result<()> {

    // 确定 base 和 quote mint/program
    let (base_mint, quote_mint, base_token_program, quote_token_program) =
        if pool_state.base_mint == WSOL_MINT {
            // WSOL 是 base mint
            (
                ctx.accounts.wsol_mint.to_account_info(),
                accounts[token_mint_index].to_account_info(),
                ctx.accounts.token_program.to_account_info(),
                accounts[token_program_index].to_account_info(),
            )
        } else {
            // Token 是 base mint
            (
                accounts[token_mint_index].to_account_info(),
                ctx.accounts.wsol_mint.to_account_info(),
                accounts[token_program_index].to_account_info(),
                ctx.accounts.token_program.to_account_info(),
            )
        };

    // 确定用户账户
    let (user_base_token_account, user_quote_token_account) =
        if pool_state.base_mint == WSOL_MINT {
            // WSOL 是 base, Token 是 quote
            (
                ctx.accounts.wsol_token_account.to_account_info(),
                accounts[token_account_index].to_account_info(),
            )
        } else {
            // Token 是 base, WSOL 是 quote
            (
                accounts[token_account_index].to_account_info(),
                ctx.accounts.wsol_token_account.to_account_info(),
            )
        };

    // 🚀 性能优化: 预分配账户数组 (Buy 有 21 个账户，包括两个统计账户)
    let account_metas: [AccountMeta; 23] = [
        AccountMeta::new_readonly(accounts[pool_state.pool_index].key(), false), // pool
        AccountMeta::new(ctx.accounts.payer.key(), true), // user (writable, signer)
        AccountMeta::new_readonly(accounts[pool_state.global_config_index].key(), false), // global_config
        AccountMeta::new_readonly(base_mint.key(), false), // base_mint
        AccountMeta::new_readonly(quote_mint.key(), false), // quote_mint
        AccountMeta::new(user_base_token_account.key(), false), // user_base_token_account
        AccountMeta::new(user_quote_token_account.key(), false), // user_quote_token_account
        AccountMeta::new(accounts[pool_state.base_vault_index].key(), false), // pool_base_token_account
        AccountMeta::new(accounts[pool_state.quote_vault_index].key(), false), // pool_quote_token_account
        AccountMeta::new_readonly(accounts[pool_state.pump_fee_wallet_index].key(), false), // protocol_fee_recipient
        AccountMeta::new(accounts[pool_state.pump_fee_wallet_ata_index].key(), false), // protocol_fee_recipient_token_account
        AccountMeta::new_readonly(base_token_program.key(), false), // base_token_program
        AccountMeta::new_readonly(quote_token_program.key(), false), // quote_token_program
        AccountMeta::new_readonly(accounts[pool_state.system_program_index].key(), false), // system_program
        AccountMeta::new_readonly(accounts[pool_state.associated_token_program_index].key(), false), // associated_token_program
        AccountMeta::new_readonly(accounts[pool_state.event_authority_index].key(), false), // event_authority
        AccountMeta::new_readonly(accounts[pool_state.program_id_index].key(), false), // program
        AccountMeta::new(accounts[pool_state.coin_creator_vault_ata_index].key(), false), // coin_creator_vault_ata
        AccountMeta::new_readonly(accounts[pool_state.coin_creator_vault_authority_index].key(), false), // coin_creator_vault_authority
        AccountMeta::new(accounts[pool_state.global_vol_accumulator_index].key(), false), // global_volume_accumulator
        AccountMeta::new(accounts[pool_state.user_vol_accumulator_index].key(), false), // user_volume_accumulator
        AccountMeta::new_readonly(accounts[pool_state.fee_config_index].key(), false), // fee_config
        AccountMeta::new_readonly(accounts[pool_state.fee_program_index].key(), false), // fee_program
    ];

    //注意 如果quote mint是wsol 那么max_quote_amount_in计算真实最大quote amount in, 如果quote mint是token 不算了，因为token余额只有那么多
    let mut max_quote_amount_in = wsol_amount;
    if pool_state.quote_mint == WSOL_MINT {
        max_quote_amount_in = pump_buy_base_input_internal(pool_state, *max_token_amount_out)?;
        // let wsol_balance = {
        //     let wsol_account_info = ctx.accounts.wsol_token_account.to_account_info();
        //     let wsol_account_data = wsol_account_info.data.borrow();    
        //     u64::from_le_bytes(
        //         wsol_account_data[64..72].try_into().unwrap()
        //     )
        // };
        // if max_quote_amount_in > wsol_balance {
        //     msg!("Stupid pump! {}, {}", max_quote_amount_in, wsol_balance);
        //     max_quote_amount_in = wsol_balance;
        //     *max_token_amount_out = *max_token_amount_out * 999 / 1000;
        // }
    }
    
    // msg!("max_quote_amount_in: {} wsol_amount: {}", max_quote_amount_in, wsol_amount);

    // 🚀 性能优化: 直接构建指令数据，避免序列化开销
    let mut instruction_data = Vec::with_capacity(24); // 8字节discriminator + 8字节base_amount_out + 8字节max_quote_amount_in
    instruction_data.extend_from_slice(&BUY_DISCRIMINATOR);
    instruction_data.extend_from_slice(&max_token_amount_out.to_le_bytes()); // base_amount_out (要买多少token)
    instruction_data.extend_from_slice(&max_quote_amount_in.to_le_bytes()); // max_quote_amount_in (最多花多少SOL)

    // 构建指令
    let buy_instruction = Instruction {
        program_id: accounts[pool_state.program_id_index].key(),
        accounts: account_metas.into(),
        data: instruction_data,
    };

    // 🚀 性能优化: 直接构建账户切片，避免Vec的堆分配
    let account_infos = &[
        accounts[pool_state.pool_index].to_account_info(),
        ctx.accounts.payer.to_account_info(),
        accounts[pool_state.global_config_index].to_account_info(),
        base_mint,
        quote_mint,
        user_base_token_account,
        user_quote_token_account,
        accounts[pool_state.base_vault_index].to_account_info(),
        accounts[pool_state.quote_vault_index].to_account_info(),
        accounts[pool_state.pump_fee_wallet_index].to_account_info(),
        accounts[pool_state.pump_fee_wallet_ata_index].to_account_info(),
        base_token_program,
        quote_token_program,
        accounts[pool_state.system_program_index].to_account_info(),
        accounts[pool_state.associated_token_program_index].to_account_info(),
        accounts[pool_state.event_authority_index].to_account_info(),
        accounts[pool_state.program_id_index].to_account_info(),
        accounts[pool_state.coin_creator_vault_ata_index].to_account_info(),
        accounts[pool_state.coin_creator_vault_authority_index].to_account_info(),
        accounts[pool_state.global_vol_accumulator_index].to_account_info(),
        accounts[pool_state.user_vol_accumulator_index].to_account_info(),
        accounts[pool_state.fee_config_index].to_account_info(),
        accounts[pool_state.fee_program_index].to_account_info(),
    ];

    // 执行 CPI 调用
    invoke(&buy_instruction, account_infos).map_err(|e| e.into())
}



/// 执行 Pump 卖出交易 (Token -> WSOL)
fn execute_pump_sell<'info>(
    pool_state: &PumpPoolState,
    token_mint_index: usize,
    token_program_index: usize,
    token_account_index: usize,
    token_amount: u64,
    min_quote_amount_out: u64,
    accounts: &[AccountInfo<'info>],
    ctx: &Context<ComparePrices<'info>>,
) -> Result<()> {
    
    // 确定 base 和 quote mint/program
    let (base_mint, quote_mint, base_token_program, quote_token_program) =
        if pool_state.base_mint == WSOL_MINT {
            // WSOL 是 base mint
            (
                ctx.accounts.wsol_mint.to_account_info(),
                accounts[token_mint_index].to_account_info(),
                ctx.accounts.token_program.to_account_info(),
                accounts[token_program_index].to_account_info(),
            )
        } else {
            // Token 是 base mint
            (
                accounts[token_mint_index].to_account_info(),
                ctx.accounts.wsol_mint.to_account_info(),
                accounts[token_program_index].to_account_info(),
                ctx.accounts.token_program.to_account_info(),
            )
        };

    // 确定用户账户
    let (user_base_token_account, user_quote_token_account) =
        if pool_state.base_mint == WSOL_MINT {
            // WSOL 是 base, Token 是 quote
            (
                ctx.accounts.wsol_token_account.to_account_info(),
                accounts[token_account_index].to_account_info(),
            )
        } else {
            // Token 是 base, WSOL 是 quote
            (
                accounts[token_account_index].to_account_info(),
                ctx.accounts.wsol_token_account.to_account_info(),
            )
        };

    // 🚀 性能优化: 预分配账户数组 (Sell 有 19 个账户，比 Buy 少了两个统计账户)
    let account_metas: [AccountMeta; 21] = [
        AccountMeta::new_readonly(accounts[pool_state.pool_index].key(), false), // pool
        AccountMeta::new(ctx.accounts.payer.key(), true), // user (writable, signer)
        AccountMeta::new_readonly(accounts[pool_state.global_config_index].key(), false), // global_config
        AccountMeta::new_readonly(base_mint.key(), false), // base_mint
        AccountMeta::new_readonly(quote_mint.key(), false), // quote_mint
        AccountMeta::new(user_base_token_account.key(), false), // user_base_token_account
        AccountMeta::new(user_quote_token_account.key(), false), // user_quote_token_account
        AccountMeta::new(accounts[pool_state.base_vault_index].key(), false), // pool_base_token_account
        AccountMeta::new(accounts[pool_state.quote_vault_index].key(), false), // pool_quote_token_account
        AccountMeta::new_readonly(accounts[pool_state.pump_fee_wallet_index].key(), false), // protocol_fee_recipient
        AccountMeta::new(accounts[pool_state.pump_fee_wallet_ata_index].key(), false), // protocol_fee_recipient_token_account
        AccountMeta::new_readonly(base_token_program.key(), false), // base_token_program
        AccountMeta::new_readonly(quote_token_program.key(), false), // quote_token_program
        AccountMeta::new_readonly(accounts[pool_state.system_program_index].key(), false), // system_program
        AccountMeta::new_readonly(accounts[pool_state.associated_token_program_index].key(), false), // associated_token_program
        AccountMeta::new_readonly(accounts[pool_state.event_authority_index].key(), false), // event_authority
        AccountMeta::new_readonly(accounts[pool_state.program_id_index].key(), false), // program
        AccountMeta::new(accounts[pool_state.coin_creator_vault_ata_index].key(), false), // coin_creator_vault_ata
        AccountMeta::new_readonly(accounts[pool_state.coin_creator_vault_authority_index].key(), false), // coin_creator_vault_authority
        AccountMeta::new_readonly(accounts[pool_state.fee_config_index].key(), false), // fee_config
        AccountMeta::new_readonly(accounts[pool_state.fee_program_index].key(), false), // fee_program
    ];



    // 🚀 性能优化: 直接构建指令数据
    let mut instruction_data = Vec::with_capacity(24); // 8字节discriminator + 8字节base_amount_in + 8字节min_quote_amount_out
    instruction_data.extend_from_slice(&SELL_DISCRIMINATOR);
    instruction_data.extend_from_slice(&token_amount.to_le_bytes()); // base_amount_in (要卖多少token)
    instruction_data.extend_from_slice(&min_quote_amount_out.to_le_bytes()); // min_quote_amount_out (最少得到多少SOL)

    // 构建指令
    let sell_instruction = Instruction {
        program_id: accounts[pool_state.program_id_index].key(),
        accounts: account_metas.into(),
        data: instruction_data,
    };

    // 🚀 性能优化: 直接构建账户切片
    let account_infos = &[
        accounts[pool_state.pool_index].to_account_info(),
        ctx.accounts.payer.to_account_info(),
        accounts[pool_state.global_config_index].to_account_info(),
        base_mint,
        quote_mint,
        user_base_token_account,
        user_quote_token_account,
        accounts[pool_state.base_vault_index].to_account_info(),
        accounts[pool_state.quote_vault_index].to_account_info(),
        accounts[pool_state.pump_fee_wallet_index].to_account_info(),
        accounts[pool_state.pump_fee_wallet_ata_index].to_account_info(),
        base_token_program,
        quote_token_program,
        accounts[pool_state.system_program_index].to_account_info(),
        accounts[pool_state.associated_token_program_index].to_account_info(),
        accounts[pool_state.event_authority_index].to_account_info(),
        accounts[pool_state.program_id_index].to_account_info(),
        accounts[pool_state.coin_creator_vault_ata_index].to_account_info(),
        accounts[pool_state.coin_creator_vault_authority_index].to_account_info(),
        accounts[pool_state.fee_config_index].to_account_info(),
        accounts[pool_state.fee_program_index].to_account_info(),
    ];

    // 执行 CPI 调用
    invoke(&sell_instruction, account_infos).map_err(|e| e.into())
}


/// 执行 Pump AMM 交易 (买入或卖出)
pub fn execute_pump_mid_swap<'info>(
    pool_state: &PumpPoolState,
    optimization_result: &OptimizationResult,
    trade_amount: u64,
    max_or_min_amount_out: u64,
    accounts: &[AccountInfo<'info>],
    ctx: &Context<ComparePrices<'info>>,
) -> Result<()> {
    
    // 🚀 提前组装核心账户信息，避免重复计算
    let token1_mint_key = accounts[optimization_result.token1_mint_index].key().to_bytes();
  
    // 判断交易方向：token1是base还是quote
    let is_token1_base = pool_state.base_mint == token1_mint_key;
    
    // 根据token1是base还是quote，确定对应的账户
    let (base_mint, quote_mint, user_base_token_account, user_quote_token_account, base_token_program, quote_token_program) = if is_token1_base {
        // token1是base
        (
            accounts[optimization_result.token1_mint_index].to_account_info(), // token1账户就是base账户
            accounts[optimization_result.token2_mint_index.unwrap()].to_account_info(), // token2账户是quote账户
            accounts[optimization_result.token1_account_index].to_account_info(), // token1账户就是base账户
            accounts[optimization_result.token2_account_index.unwrap()].to_account_info(), // token2账户是quote账户
            accounts[optimization_result.token1_program_index].to_account_info(), // token1 program是base program
            accounts[optimization_result.token2_program_index.unwrap()].to_account_info(), // token2 program是quote program
        )
    } else {
        // token1是quote
        (
            accounts[optimization_result.token2_mint_index.unwrap()].to_account_info(), // token2账户是base账户
            accounts[optimization_result.token1_mint_index].to_account_info(), // token1账户就是quote账户
            accounts[optimization_result.token2_account_index.unwrap()].to_account_info(), // token2账户是base账户
            accounts[optimization_result.token1_account_index].to_account_info(), // token1账户就是quote账户
            accounts[optimization_result.token2_program_index.unwrap()].to_account_info(), // token2 program是base program
            accounts[optimization_result.token1_program_index].to_account_info(), // token1 program是quote program
        )
    };
   
    if pool_state.quote_mint == accounts[optimization_result.token1_mint_index].key().to_bytes(){
        // 🚀 内存优化：使用Vec避免大型栈数组
        let mut account_metas = Vec::with_capacity(23);
        account_metas.extend_from_slice(&[
            AccountMeta::new_readonly(accounts[pool_state.pool_index].key(), false),
            AccountMeta::new(ctx.accounts.payer.key(), true),
            AccountMeta::new_readonly(accounts[pool_state.global_config_index].key(), false),
            AccountMeta::new_readonly(base_mint.key(), false),
            AccountMeta::new_readonly(quote_mint.key(), false),
            AccountMeta::new(user_base_token_account.key(), false),
            AccountMeta::new(user_quote_token_account.key(), false),
            AccountMeta::new(accounts[pool_state.base_vault_index].key(), false),
            AccountMeta::new(accounts[pool_state.quote_vault_index].key(), false),
            AccountMeta::new_readonly(accounts[pool_state.pump_fee_wallet_index].key(), false),
            AccountMeta::new(accounts[pool_state.pump_fee_wallet_ata_index].key(), false),
            AccountMeta::new_readonly(base_token_program.key(), false),
            AccountMeta::new_readonly(quote_token_program.key(), false),
            AccountMeta::new_readonly(accounts[pool_state.system_program_index].key(), false),
            AccountMeta::new_readonly(accounts[pool_state.associated_token_program_index].key(), false),
            AccountMeta::new_readonly(accounts[pool_state.event_authority_index].key(), false),
            AccountMeta::new_readonly(accounts[pool_state.program_id_index].key(), false),
            AccountMeta::new(accounts[pool_state.coin_creator_vault_ata_index].key(), false),
            AccountMeta::new_readonly(accounts[pool_state.coin_creator_vault_authority_index].key(), false),
            AccountMeta::new(accounts[pool_state.global_vol_accumulator_index].key(), false),
            AccountMeta::new(accounts[pool_state.user_vol_accumulator_index].key(), false),
            AccountMeta::new_readonly(accounts[pool_state.fee_config_index].key(), false),
            AccountMeta::new_readonly(accounts[pool_state.fee_program_index].key(), false),
        ]);
    
        
        // 🚀 性能优化: 直接构建指令数据，避免序列化开销
        let mut instruction_data = Vec::with_capacity(24); // 8字节discriminator + 8字节base_amount_out + 8字节max_quote_amount_in
        instruction_data.extend_from_slice(&BUY_DISCRIMINATOR);
        instruction_data.extend_from_slice(&max_or_min_amount_out.to_le_bytes()); // base_amount_out (要买多少token)
        instruction_data.extend_from_slice(&trade_amount.to_le_bytes()); // max_quote_amount_in (最多花多少SOL)
    
        // 构建指令
        let buy_instruction = Instruction {
            program_id: accounts[pool_state.program_id_index].key(),
            accounts: account_metas,
            data: instruction_data,
        };
    
        // 🚀 性能优化: 直接构建账户切片，避免Vec的堆分配
        let account_infos = &[
            accounts[pool_state.pool_index].to_account_info(),
            ctx.accounts.payer.to_account_info(),
            accounts[pool_state.global_config_index].to_account_info(),
            base_mint,
            quote_mint,
            user_base_token_account,
            user_quote_token_account,
            accounts[pool_state.base_vault_index].to_account_info(),
            accounts[pool_state.quote_vault_index].to_account_info(),
            accounts[pool_state.pump_fee_wallet_index].to_account_info(),
            accounts[pool_state.pump_fee_wallet_ata_index].to_account_info(),
            base_token_program,
            quote_token_program,
            accounts[pool_state.system_program_index].to_account_info(),
            accounts[pool_state.associated_token_program_index].to_account_info(),
            accounts[pool_state.event_authority_index].to_account_info(),
            accounts[pool_state.program_id_index].to_account_info(),
            accounts[pool_state.coin_creator_vault_ata_index].to_account_info(),
            accounts[pool_state.coin_creator_vault_authority_index].to_account_info(),
            accounts[pool_state.global_vol_accumulator_index].to_account_info(),
            accounts[pool_state.user_vol_accumulator_index].to_account_info(),
            accounts[pool_state.fee_config_index].to_account_info(),
            accounts[pool_state.fee_program_index].to_account_info(),
        ];
    
        // 执行 CPI 调用
        invoke(&buy_instruction, account_infos).map_err(|e| e.into())
    }else{
        // 🚀 内存优化：使用Vec避免大型栈数组
        let mut account_metas = Vec::with_capacity(21);
        account_metas.extend_from_slice(&[
            AccountMeta::new_readonly(accounts[pool_state.pool_index].key(), false),
            AccountMeta::new(ctx.accounts.payer.key(), true),
            AccountMeta::new_readonly(accounts[pool_state.global_config_index].key(), false),
            AccountMeta::new_readonly(base_mint.key(), false),
            AccountMeta::new_readonly(quote_mint.key(), false),
            AccountMeta::new(user_base_token_account.key(), false),
            AccountMeta::new(user_quote_token_account.key(), false),
            AccountMeta::new(accounts[pool_state.base_vault_index].key(), false),
            AccountMeta::new(accounts[pool_state.quote_vault_index].key(), false),
            AccountMeta::new_readonly(accounts[pool_state.pump_fee_wallet_index].key(), false),
            AccountMeta::new(accounts[pool_state.pump_fee_wallet_ata_index].key(), false),
            AccountMeta::new_readonly(base_token_program.key(), false),
            AccountMeta::new_readonly(quote_token_program.key(), false),
            AccountMeta::new_readonly(accounts[pool_state.system_program_index].key(), false),
            AccountMeta::new_readonly(accounts[pool_state.associated_token_program_index].key(), false),
            AccountMeta::new_readonly(accounts[pool_state.event_authority_index].key(), false),
            AccountMeta::new_readonly(accounts[pool_state.program_id_index].key(), false),
            AccountMeta::new(accounts[pool_state.coin_creator_vault_ata_index].key(), false),
            AccountMeta::new_readonly(accounts[pool_state.coin_creator_vault_authority_index].key(), false),
            AccountMeta::new_readonly(accounts[pool_state.fee_config_index].key(), false),
            AccountMeta::new_readonly(accounts[pool_state.fee_program_index].key(), false),
        ]);



        // 🚀 性能优化: 直接构建指令数据
        let mut instruction_data = Vec::with_capacity(24); // 8字节discriminator + 8字节base_amount_in + 8字节min_quote_amount_out
        instruction_data.extend_from_slice(&SELL_DISCRIMINATOR);
        instruction_data.extend_from_slice(&trade_amount.to_le_bytes()); // base_amount_in (要卖多少token)
        instruction_data.extend_from_slice(&0u64.to_le_bytes()); // min_quote_amount_out (最少得到多少SOL)

        // 构建指令
        let sell_instruction = Instruction {
            program_id: accounts[pool_state.program_id_index].key(),
            accounts: account_metas,
            data: instruction_data,
        };

        // 🚀 性能优化: 直接构建账户切片
        let account_infos = &[
            accounts[pool_state.pool_index].to_account_info(),
            ctx.accounts.payer.to_account_info(),
            accounts[pool_state.global_config_index].to_account_info(),
            base_mint,
            quote_mint,
            user_base_token_account,
            user_quote_token_account,
            accounts[pool_state.base_vault_index].to_account_info(),
            accounts[pool_state.quote_vault_index].to_account_info(),
            accounts[pool_state.pump_fee_wallet_index].to_account_info(),
            accounts[pool_state.pump_fee_wallet_ata_index].to_account_info(),
            base_token_program,
            quote_token_program,
            accounts[pool_state.system_program_index].to_account_info(),
            accounts[pool_state.associated_token_program_index].to_account_info(),
            accounts[pool_state.event_authority_index].to_account_info(),
            accounts[pool_state.program_id_index].to_account_info(),
            accounts[pool_state.coin_creator_vault_ata_index].to_account_info(),
            accounts[pool_state.coin_creator_vault_authority_index].to_account_info(),
            accounts[pool_state.fee_config_index].to_account_info(),
            accounts[pool_state.fee_program_index].to_account_info(),
        ];

        // 执行 CPI 调用
        invoke(&sell_instruction, account_infos).map_err(|e| e.into())
        }
}

