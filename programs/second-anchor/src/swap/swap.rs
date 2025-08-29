use crate::comparison::GlobalArbitrageAnalysis;
use crate::ComparePrices;
use anchor_lang::prelude::*;

use crate::swap::{cpmm::execute_cpmm_swap, dlmm::execute_dlmm_swap, dammv2::execute_dammv2_swap, pump::execute_pump_swap};
use crate::optimalamt::OptimizationResult;
use crate::comparison::ParsedPoolState;


/// 执行套利交易的主要函数
pub fn execute_arbitrage_swaps<'a, 'b, 'c, 'info>(
    analysis: &GlobalArbitrageAnalysis,
    optimization_result: &OptimizationResult,
    accounts: &[AccountInfo<'info>],
    ctx: &Context<'a, 'b, 'c, 'info, ComparePrices<'info>>,
) -> Result<()> {
  
    // msg!("optimization_result: {:?}", optimization_result);

    // 第一步：在买入池购买token
    execute_buy_swap(
        analysis,
        optimization_result.optimal_wsol_amount,
        optimization_result.max_mint_amount_out,
        accounts,
        ctx,
    )?;

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

    //获取当前token余额
    // let payer_token_account = &accounts[analysis.mint_token_account_index.unwrap()];
    // let token_account_data =
    //     TokenAccount::try_deserialize(&mut &payer_token_account.data.borrow()[..])?;

    // if token_account_data.amount != optimization_result.max_mint_amount_out {
    //     msg!("当前token余额: {}", token_account_data.amount);
    //     msg!(
    //         "计算的token余额: {}",
    //         optimization_result.max_mint_amount_out
    //     );
    //     msg!("计算有误！");
    //     return Err(ErrorCode::WrongCalculation.into());
    // }

    // 第二步：在卖出池卖出token
    execute_sell_swap(analysis, optimization_result.max_mint_amount_out, accounts, ctx)?;

    Ok(())
}

/// 执行买入交易
fn execute_buy_swap<'info>(
    analysis: &GlobalArbitrageAnalysis,
    wsol_amount: u64,
    max_or_min_amount_out: u64,
    accounts: &[AccountInfo<'info>],
    ctx: &Context<ComparePrices<'info>>,
) -> Result<()> {
    match analysis.buy_pool_state.as_ref().unwrap() {
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
            true,
        ),
    }
}

/// 执行卖出交易
fn execute_sell_swap<'info>(
    analysis: &GlobalArbitrageAnalysis,
    sell_token_amount: u64,
    accounts: &[AccountInfo<'info>],
    ctx: &Context<ComparePrices<'info>>,
) -> Result<()> {
    match analysis.sell_pool_state.as_ref().unwrap() {
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
            sell_token_amount,
            accounts,
            ctx,
            false,
        ),
        ParsedPoolState::CPMM { state } => execute_cpmm_swap(
            state,
            analysis.best_token_mint_index.unwrap(),
            analysis.token_program_index.unwrap(),
            analysis.mint_token_account_index.unwrap(),
            sell_token_amount,
            accounts,
            ctx,
            false,
        ),
        ParsedPoolState::DAMMV2 { state } => execute_dammv2_swap(
            state,
            analysis.best_token_mint_index.unwrap(),
            analysis.token_program_index.unwrap(),
            analysis.mint_token_account_index.unwrap(),
            sell_token_amount,
            accounts,
            ctx,
            false,
        ),
        ParsedPoolState::PUMP { state } => execute_pump_swap(
            state,
            analysis.best_token_mint_index.unwrap(),
            analysis.token_program_index.unwrap(),
            analysis.mint_token_account_index.unwrap(),
            sell_token_amount,
            0, // 卖出时设置为0，避免滑点问题
            accounts,
            ctx,
            false,
        ),
    }
}
