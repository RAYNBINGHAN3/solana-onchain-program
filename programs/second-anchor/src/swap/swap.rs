use crate::comparison::GlobalArbitrageAnalysis;
use crate::{dex, ComparePrices};
use anchor_lang::prelude::*;

use crate::comparison::ParsedPoolState;
use crate::optimalamt::OptimizationResult;
use crate::swap::{
    clmm::execute_clmm_swap, cpmm::execute_cpmm_swap, dammv2::execute_dammv2_swap, 
    dlmm::execute_dlmm_swap, pump::execute_pump_swap, raydium::execute_raydium_swap,
    whirlpool::execute_whirlpool_swap,
};
use crate::utils::errors::ErrorCode;

pub struct SwapResult {
    pub wsol_in: u64,
    // pub token_out: u64,
    pub profit: u64,
}


/// 执行套利交易的主要函数
pub fn execute_arbitrage_swaps<'a, 'b, 'c, 'info>(
    analysis: &mut GlobalArbitrageAnalysis,
    optimization_result: &OptimizationResult,
    accounts: &[AccountInfo<'info>],
    ctx: &Context<'a, 'b, 'c, 'info, ComparePrices<'info>>,
    initial_wsol_balance: u64,
    min_profit: u32,
    wsol_mint_key: Pubkey,
) -> Result<SwapResult> {
    
    // 第一步：在买入池购买token
    execute_buy_swap(
        analysis,
        optimization_result.optimal_wsol_amount,
        optimization_result.max_mint_amount_out,
        accounts,
        ctx,
        wsol_mint_key,
    )?;

    // 🚀 内存优化：买入完成后释放buy_pool_state内存，减少堆内存使用
    analysis.buy_pool_state = None;

    // //如果买入是pump 校验token余额
    // let mut token_in = optimization_result.max_mint_amount_out;
    // match analysis.buy_pool_state.as_ref().unwrap() {
    //     ParsedPoolState::PUMP{ state} => {
    //         let token_account_data =
    //             TokenAccount::try_deserialize(&mut &accounts[analysis.mint_token_account_index.unwrap()].data.borrow()[..])?;
    //         if token_account_data.amount != optimization_result.max_mint_amount_out {
    //             msg!("实际token: {}", token_account_data.amount);
    //             msg!("计算token: {}", token_in);
    //             token_in = token_account_data.amount;
    //         }
    //     }
    //     _ => {}
    // }
    

    
    // 直接读取token账户数据的amount字段 (位置相同，兼容Token和Token2022)
    let token_balance = {
        let payer_token_account = &accounts[analysis.mint_token_account_index.unwrap()];
        let account_data = payer_token_account.data.borrow();       
        // Token账户结构: mint(32) + owner(32) + amount(8) + ...
        u64::from_le_bytes(
            account_data[64..72].try_into().map_err(|_| ErrorCode::InvalidAccount)?
        )
    };

    // if token_account_data.amount != optimization_result.max_mint_amount_out {
    //     msg!("当前token余额: {}", token_account_data.amount);
    //     msg!(
    //         "计算的token余额: {}",
    //         optimization_result.max_mint_amount_out
    //     );
    //     // msg!("计算有误！");
    //     // return Err(ErrorCode::WrongCalculation.into());
    // }

    // 第二步：在卖出池卖出token
    execute_sell_swap(
        analysis,
        token_balance,
        accounts,
        ctx,
        optimization_result.total_wsol_out, // pupm sol为base mint的情况 那么final_wsol_out就是base amount
        wsol_mint_key,
    )?;

    // 🚀 内存优化：卖出完成后释放sell_pool_state内存，进一步减少堆内存使用
    analysis.sell_pool_state = None;

    //检验wsol余额手动获取实时余额
    let after_wsol_balance = {
        let wsol_account_info = ctx.accounts.wsol_token_account.to_account_info();
        let wsol_account_data = wsol_account_info.data.borrow();    
        u64::from_le_bytes(
            wsol_account_data[64..72].try_into().map_err(|_| ErrorCode::InvalidAccount)?
        )
    };
    // msg!("wsol a: {}", after_wsol_balance);
    let real_profit = after_wsol_balance.saturating_sub(initial_wsol_balance).saturating_sub(min_profit as u64);
    if real_profit <= 4 {
        return Err(ErrorCode::NoProfit.into());
    }

    Ok(SwapResult {
        wsol_in: optimization_result.optimal_wsol_amount,
        // token_out: token_balance,
        profit: real_profit,
    })
}

/// 执行买入交易
fn execute_buy_swap<'info>(
    analysis: &mut GlobalArbitrageAnalysis,
    wsol_amount: u64,
    max_or_min_amount_out: u64,
    accounts: &[AccountInfo<'info>],
    ctx: &Context<ComparePrices<'info>>,
    wsol_mint_key: Pubkey,
) -> Result<()> {
    match analysis.buy_pool_state.as_ref().unwrap().as_ref() {
        ParsedPoolState::DLMM {
            state,
            reserve_x_index,
            reserve_y_index,
            bin_array_minus_1_index,
            bin_array_0_index,
            bin_array_1_index,
        } => execute_dlmm_swap(
            state,
            analysis.best_token_mint_index.unwrap(),
            analysis.token_program_index.unwrap(),
            analysis.mint_token_account_index.unwrap(),
            *bin_array_minus_1_index,
            *bin_array_0_index,
            *bin_array_1_index,
            *reserve_x_index,
            *reserve_y_index,
            wsol_amount,
            accounts,
            ctx,
            wsol_mint_key,
            true,
        ),
        ParsedPoolState::CPMM { state } => execute_cpmm_swap(
            state,
            analysis.best_token_mint_index.unwrap(),
            analysis.token_program_index.unwrap(),
            analysis.mint_token_account_index.unwrap(),
            wsol_amount,
            accounts,
            ctx,
            wsol_mint_key,
            true,
        ),
        ParsedPoolState::DAMMV2 { state } => execute_dammv2_swap(
            state,
            analysis.best_token_mint_index.unwrap(),
            analysis.token_program_index.unwrap(),
            analysis.mint_token_account_index.unwrap(),
            wsol_amount,
            accounts,
            ctx,
            wsol_mint_key,
            true,
        ),
        ParsedPoolState::PUMP { state } => execute_pump_swap(
            state,
            analysis.best_token_mint_index.unwrap(),
            analysis.token_program_index.unwrap(),
            analysis.mint_token_account_index.unwrap(),
            wsol_amount,
            max_or_min_amount_out,
            accounts,
            ctx,
            wsol_mint_key,
            true,
        ),
        ParsedPoolState::RAYDIUM { state } => execute_raydium_swap(
            state,
            analysis.token_program_index.unwrap(),
            analysis.mint_token_account_index.unwrap(),
            wsol_amount,
            accounts,
            ctx,
            true,
        ),
        ParsedPoolState::CLMM { state, .. } => {
      
            execute_clmm_swap(
            state,
            analysis.best_token_mint_index.unwrap(),
            analysis.mint_token_account_index.unwrap(),
            wsol_amount,
            accounts,
            ctx,
            true,
        )
    },
        ParsedPoolState::WHIRLPOOL { state } => execute_whirlpool_swap(
            state,
            analysis.token_program_index.unwrap(),  
            analysis.best_token_mint_index.unwrap(),
            analysis.mint_token_account_index.unwrap(),
            wsol_amount,
            accounts,
            ctx,
            true,
        ),
    }
}

/// 执行卖出交易
fn execute_sell_swap<'info>(
    analysis: &GlobalArbitrageAnalysis,
    token_amount_balance: u64,
    accounts: &[AccountInfo<'info>],
    ctx: &Context<ComparePrices<'info>>,
    final_wsol_out: u64,
    wsol_mint_key: Pubkey,
) -> Result<()> {
    match analysis.sell_pool_state.as_ref().unwrap().as_ref() {
        ParsedPoolState::DLMM {
            state,
            reserve_x_index,
            reserve_y_index,
            bin_array_minus_1_index,
            bin_array_0_index,
            bin_array_1_index,
        } => execute_dlmm_swap(
            state,
            analysis.best_token_mint_index.unwrap(),
            analysis.token_program_index.unwrap(),
            analysis.mint_token_account_index.unwrap(),
            *bin_array_minus_1_index,
            *bin_array_0_index,
            *bin_array_1_index,
            *reserve_x_index,
            *reserve_y_index,
            token_amount_balance,  //dlmm不准暂时用 token_amount_balance
            accounts,
            ctx,
            wsol_mint_key,
            false,
        ),
        ParsedPoolState::CPMM { state } => execute_cpmm_swap(
            state,
            analysis.best_token_mint_index.unwrap(),
            analysis.token_program_index.unwrap(),
            analysis.mint_token_account_index.unwrap(),
            token_amount_balance,
            accounts,
            ctx,
            wsol_mint_key,
            false,
        ),
        ParsedPoolState::DAMMV2 { state } => execute_dammv2_swap(
            state,
            analysis.best_token_mint_index.unwrap(),
            analysis.token_program_index.unwrap(),
            analysis.mint_token_account_index.unwrap(),
            token_amount_balance,
            accounts,
            ctx,
            wsol_mint_key,
            false,
        ),
        ParsedPoolState::PUMP { state } => {
            // 缓存wsol_mint_key避免重复调用
            let mut current_wsol_out = final_wsol_out;
            
            // 表示为is_swap_dir=1切quote mint不是wsol，此时为买入base wsol， 那边提供不了base_amount,需要使用token_amount_balance计算base_amount of wsol     
            if current_wsol_out == 0 && state.base_mint == wsol_mint_key {
                // 提前检查token_amount_balance避免不必要的函数调用
                current_wsol_out = dex::pump::pump_quote_exact_input_token(
                    state, 
                    wsol_mint_key, 
                    token_amount_balance, 
                    &ctx.remaining_accounts[analysis.mint_token_account_index.unwrap()]
                )?.saturating_sub(100); //留点余地 防止实际max quote in大于token 余额
            }
            
            execute_pump_swap(
            state,
            analysis.best_token_mint_index.unwrap(),
            analysis.token_program_index.unwrap(),
            analysis.mint_token_account_index.unwrap(),
            token_amount_balance,
            current_wsol_out, //当wsol是base mint时，current_wsol_out需要提供给pump臭傻逼 
            accounts,
            ctx,
            wsol_mint_key,
            false,
        )
    },
        ParsedPoolState::RAYDIUM { state } => execute_raydium_swap(
            state,
            analysis.token_program_index.unwrap(),
            analysis.mint_token_account_index.unwrap(),
            token_amount_balance,
            accounts,
            ctx,
            false,
        ),
        ParsedPoolState::CLMM { state, .. } => execute_clmm_swap(
            state,
            analysis.best_token_mint_index.unwrap(),
            analysis.mint_token_account_index.unwrap(),
            token_amount_balance,
            accounts,
            ctx,
            false,
        ),
        ParsedPoolState::WHIRLPOOL { state } => execute_whirlpool_swap(
            state,
            analysis.token_program_index.unwrap(),
            analysis.best_token_mint_index.unwrap(),
            analysis.mint_token_account_index.unwrap(),
            token_amount_balance,
            accounts,
            ctx,
            false,
        ),
    }
}
