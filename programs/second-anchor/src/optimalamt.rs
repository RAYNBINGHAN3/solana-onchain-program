use crate::comparison::ParsedPoolState;
use crate::dex::cpmm::{cpmm_quote_exact_input_token, cpmm_quote_exact_input_wsol};
use crate::dex::dammv2::{dammv2_quote_exact_input_token, dammv2_quote_exact_input_wsol};
use crate::dex::dlmm::{
    dlmm_quote_exact_input_token_optimized, dlmm_quote_exact_input_wsol_optimized,
    parse_dlmm_bin_arrays_optimized,
};
use crate::dex::pump::{pump_quote_exact_input_token, pump_quote_exact_input_wsol};
use crate::utils::errors::ErrorCode;
use crate::utils::u128x128_math::{safe_mul_div_cast, Rounding};
use crate::utils::u64x64_math::ONE;
use anchor_lang::prelude::*;
use std::cmp::min;

/// 套利优化结果
#[derive(Debug)]
pub struct OptimizationResult {
    pub optimal_wsol_amount: u64,
    pub max_profit: i64,
    pub max_mint_amount_out: u64,
    pub iterations: u8,
}

/// 优化的黄金分割法寻找最优买入SOL数量
/// 基于套利利润函数单峰性质的智能优化算法
pub fn find_optimal_wsol_amount_golden_section(
    buy_pool_state: &ParsedPoolState,
    sell_pool_state: &ParsedPoolState,
    wsol_mint: Pubkey,
    max_wsol_balance: u64,
    accounts: &[AccountInfo],
    max_profit_ratio: u128,
    min_profit: u32,
) -> Result<OptimizationResult> {
    let mut precision = 100_000; // 0.0001 SOL
    require!(max_wsol_balance >= precision, ErrorCode::NoProfit);

    let is_dlmm_involved = matches!(buy_pool_state, ParsedPoolState::DLMM { .. })
        || matches!(sell_pool_state, ParsedPoolState::DLMM { .. });

    // 首先提取池子价格用于后续计算
    let buy_pool_price = match buy_pool_state {
        ParsedPoolState::CPMM { state } => state.price,
        ParsedPoolState::DLMM { state, .. } => state.price,
        ParsedPoolState::DAMMV2 { state, .. } => state.price,
        ParsedPoolState::PUMP { state, .. } => state.price,
    };

    let sell_pool_price = match sell_pool_state {
        ParsedPoolState::CPMM { state } => state.price,
        ParsedPoolState::DLMM { state, .. } => state.price,
        ParsedPoolState::DAMMV2 { state, .. } => state.price,
        ParsedPoolState::PUMP { state, .. } => state.price,
    };

    let (buy_ordered_bins, sell_ordered_bins) = if is_dlmm_involved {
        get_ordered_bins_for_pools(
            accounts,
            buy_pool_state,
            sell_pool_state,
            wsol_mint,
            buy_pool_price,
            sell_pool_price,
        )?
    } else {
        (None, None)
    };

    let max_effective_input_buy = match buy_pool_state {
        ParsedPoolState::CPMM { state } => {
            let wsol_reserve = if state.token0_mint == wsol_mint {
                state.token0_reserve
            } else {
                state.token1_reserve
            };

            safe_mul_div_cast(wsol_reserve as u128, max_profit_ratio, ONE, Rounding::Down) as u64
            //后续版本优化算法
        }
        ParsedPoolState::DLMM { .. } => {
            if let Some(ref bins) = buy_ordered_bins {
                bins.iter()
                    .map(|(_, bin)| bin.max_amount_in_with_fees)
                    .sum::<u64>()
            } else {
                0
            }
        }
        ParsedPoolState::DAMMV2 { state, .. } => {
            // DAMMV2买入：使用WSOL vault容量的5%
            let wsol_vault_index = if state.token_a_mint == wsol_mint {
                state.token_a_vault_index
            } else {
                state.token_b_vault_index
            };
            let wsol_vault = &accounts[wsol_vault_index];
            let wsol_vault_data = wsol_vault.data.borrow();
            let wsol_balance =
                u64::from_le_bytes(wsol_vault_data[64..72].try_into().unwrap_or([0; 8]));
            safe_mul_div_cast(wsol_balance as u128, max_profit_ratio, ONE, Rounding::Down) as u64
            //后续版本优化算法
        }
        ParsedPoolState::PUMP { state, .. } => {
            // Pump买入：使用WSOL reserve容量的5%
            let wsol_reserve = if state.quote_mint == wsol_mint {
                state.quote_reserve
            } else {
                state.base_reserve
            };
            safe_mul_div_cast(wsol_reserve as u128, max_profit_ratio, ONE, Rounding::Down) as u64
            //后续版本优化算法
        }
    };

    let max_effective_input_sell = match sell_pool_state {
        ParsedPoolState::CPMM { state } => {
            let wsol_reserve = if state.token0_mint == wsol_mint {
                state.token0_reserve
            } else {
                state.token1_reserve
            };

            safe_mul_div_cast(wsol_reserve as u128, max_profit_ratio, ONE, Rounding::Down) as u64
            //后续版本优化算法
        }
        ParsedPoolState::DLMM { state, .. } => {
            if let Some(ref bins) = sell_ordered_bins {
                if state.token_y_mint == wsol_mint {
                    bins.iter().map(|(_, bin)| bin.amount_y).sum::<u64>()
                } else {
                    bins.iter().map(|(_, bin)| bin.amount_x).sum::<u64>()
                }
            } else {
                0
            }
        }
        ParsedPoolState::DAMMV2 { state, .. } => {
            let wsol_vault_index = if state.token_a_mint == wsol_mint {
                state.token_a_vault_index
            } else {
                state.token_b_vault_index
            };
            let wsol_vault = &accounts[wsol_vault_index];
            let wsol_vault_data = wsol_vault.data.borrow();
            let wsol_balance =
                u64::from_le_bytes(wsol_vault_data[64..72].try_into().unwrap_or([0; 8]));
            safe_mul_div_cast(wsol_balance as u128, max_profit_ratio, ONE, Rounding::Down) as u64
            //后续版本优化算法
        }
        ParsedPoolState::PUMP { state, .. } => {
            let wsol_reserve = if state.quote_mint == wsol_mint {
                state.quote_reserve
            } else {
                state.base_reserve
            };
            safe_mul_div_cast(wsol_reserve as u128, max_profit_ratio, ONE, Rounding::Down) as u64
            //后续版本优化算法
        }
    };

    let mut left = 0;
    let mut right = min(max_effective_input_buy, max_effective_input_sell);

    msg!("D {} J {}", max_effective_input_buy as f64 / 1_000_000_000.0, max_effective_input_sell as f64 / 1_000_000_000.0);

    // 足够小的测试点
    let test_point = min(100_000, right);
    let (profit, token_amount_out) = calculate_arbitrage_profit_optimized(
        buy_pool_state,
        sell_pool_state,
        wsol_mint,
        test_point,
        &buy_ordered_bins,
        &sell_ordered_bins,
    )?;
    require!(profit > min_profit as i64, ErrorCode::NoProfit);
    if right <= 100_000 {
        return Ok(OptimizationResult {
            optimal_wsol_amount: test_point,
            max_profit: profit,
            max_mint_amount_out: token_amount_out,
            iterations: 0,
        });
    }

    // 代表有利润的订单 可计算
    let mut first_denom = 5;
    let mut max_profit = profit;
    let mut best_amount = test_point;
    let mut max_mint_amount_out = token_amount_out;

    let test_point2 = right / 160;
    if test_point < test_point2 {
        let (profit2, token_amount_out2) = calculate_arbitrage_profit_optimized(
            buy_pool_state,
            sell_pool_state,
            wsol_mint,
            test_point2,
            &buy_ordered_bins,
            &sell_ordered_bins,
        )?;

        // 在右边
        if profit2 > profit {
            max_profit = profit2;
            best_amount = test_point2;
            max_mint_amount_out = token_amount_out2;
            left = test_point2;
        } else {
            right = test_point2;
            if right > 16_000_000 {
                // first_denom *= right / 16_000_000;
                first_denom = 8;
            }
            precision = 10_000;
        }
    }


    // 二分法精确搜索
    let mut iterations = 0u8;
    while right - left > precision && iterations < 6 {
        let mid = if iterations == 0 {
            (left + right) / first_denom
        } else {
            (left + right) / 2
        };

        let (mid_profit, mid_token_out) = calculate_arbitrage_profit_optimized(
            buy_pool_state,
            sell_pool_state,
            wsol_mint,
            mid,
            &buy_ordered_bins,
            &sell_ordered_bins,
        )?;

        iterations += 1;

        if mid_profit > max_profit {
            max_profit = mid_profit;
            best_amount = mid;
            max_mint_amount_out = mid_token_out;
        }

        if mid_profit < 0 {
            // 峰值在左边
            right = mid;
        } else {
            // 向左探索
            if mid > best_amount {
                right = mid;
            } else {
                left = mid;
            }
        }
    }

    //helius sender
    require!(max_profit > min_profit as i64, ErrorCode::NoProfit);

    best_amount = min(best_amount, max_wsol_balance);

    Ok(OptimizationResult {
        optimal_wsol_amount: best_amount,
        max_profit,
        max_mint_amount_out,
        iterations,
    })
}

/// 使用已知的池价格来限制bin解析范围，大幅减少CU消耗
pub fn get_ordered_bins_for_pools(
    accounts: &[AccountInfo],
    buy_pool_state: &ParsedPoolState,
    sell_pool_state: &ParsedPoolState,
    wsol_mint: Pubkey,
    buy_pool_price: u128,
    sell_pool_price: u128,
) -> Result<(
    Option<Vec<(i32, crate::dex::dlmm::BinState)>>,
    Option<Vec<(i32, crate::dex::dlmm::BinState)>>,
)> {
    let buy_ordered_bins = match buy_pool_state {
        ParsedPoolState::DLMM {
            state,
            bin_array_minus_1_index,
            bin_array_0_index,
            bin_array_1_index,
            ..
        } => {
            //wsol -> token
            let bins = parse_dlmm_bin_arrays_optimized(
                state,
                accounts,
                *bin_array_minus_1_index,
                *bin_array_0_index,
                *bin_array_1_index,
                state.active_id,
                !(state.token_y_mint == wsol_mint), // WSOL输入，Token输出
                buy_pool_price,
                sell_pool_price,
                wsol_mint,
                true,
            )?;
            Some(bins)
        }
        ParsedPoolState::CPMM { .. } => None,
        ParsedPoolState::DAMMV2 { .. } => None,
        ParsedPoolState::PUMP { .. } => None,
    };

    let sell_ordered_bins = match sell_pool_state {
        ParsedPoolState::DLMM {
            state,
            bin_array_minus_1_index,
            bin_array_0_index,
            bin_array_1_index,
            ..
        } => {
            //token -> wsol
            let bins = parse_dlmm_bin_arrays_optimized(
                state,
                accounts,
                *bin_array_minus_1_index,
                *bin_array_0_index,
                *bin_array_1_index,
                state.active_id,
                state.token_y_mint == wsol_mint, // Token输入，WSOL输出
                buy_pool_price,                  // 传入买入池价格用于范围限制
                sell_pool_price,                 // 传入卖出池价格
                wsol_mint,
                false,
            )?;
            Some(bins)
        }
        ParsedPoolState::CPMM { .. } => None,
        ParsedPoolState::DAMMV2 { .. } => None,
        ParsedPoolState::PUMP { .. } => None,
    };

    Ok((buy_ordered_bins, sell_ordered_bins))
}

/// 避免在每次计算中重新解析bin arrays
fn calculate_arbitrage_profit_optimized(
    buy_pool_state: &ParsedPoolState,
    sell_pool_state: &ParsedPoolState,
    wsol_mint: Pubkey,
    wsol_amount_in: u64,
    buy_ordered_bins: &Option<Vec<(i32, crate::dex::dlmm::BinState)>>,
    sell_ordered_bins: &Option<Vec<(i32, crate::dex::dlmm::BinState)>>,
) -> Result<(i64, u64)> {
    if wsol_amount_in == 0 {
        return Ok((0, 0));
    }

    // 步骤1: 在买入池用SOL买Token
    let token_amount_out = quote_buy_token_with_wsol_optimized(
        buy_pool_state,
        wsol_mint,
        wsol_amount_in,
        buy_ordered_bins,
    )?;

    if token_amount_out == 0 {
        return Ok((0, 0));
    }

    // 步骤2: 在卖出池用Token卖出换SOL
    let wsol_amount_out = quote_sell_token_for_wsol_optimized(
        sell_pool_state,
        wsol_mint,
        token_amount_out,
        sell_ordered_bins,
    )?;

    // 计算净利润
    let profit = (wsol_amount_out as i64).saturating_sub(wsol_amount_in as i64);

    Ok((profit, token_amount_out))
}

/// 优化版买入接口 - 使用预解析的bin数据
fn quote_buy_token_with_wsol_optimized(
    pool_state: &ParsedPoolState,
    wsol_mint: Pubkey,
    wsol_amount_in: u64,
    ordered_bins: &Option<Vec<(i32, crate::dex::dlmm::BinState)>>,
) -> Result<u64> {
    match pool_state {
        ParsedPoolState::CPMM { state } => {
            let token_amount_out = cpmm_quote_exact_input_wsol(state, wsol_mint, wsol_amount_in)?;
            // msg!("买入 CPMM wsol_amount_in: {}  token_amount_out: {}", wsol_amount_in as f64 / 1_000_000_000.0, token_amount_out);
            Ok(token_amount_out)
        }
        ParsedPoolState::DLMM { state, .. } => {
            // 使用预解析的bin数据
            if let Some(bins) = ordered_bins {
                let token_amount_out =
                    dlmm_quote_exact_input_wsol_optimized(state, wsol_mint, wsol_amount_in, bins)?;
                // msg!("买入 DLMM wsol_amount_in: {}  token_amount_out: {}", wsol_amount_in as f64 / 1_000_000_000.0, token_amount_out);
                Ok(token_amount_out)
            } else {
                Err(ErrorCode::NoProfit.into())
            }
        }
        ParsedPoolState::DAMMV2 { state } => {
            let current_point = match state.activation_type {
                0 => Clock::get()?.slot,
                1 => Clock::get()?.unix_timestamp as u64,
                _ => return Err(ErrorCode::InvalidActivationType.into()),
            };

            let token_amount_out =
                dammv2_quote_exact_input_wsol(state, wsol_mint, wsol_amount_in, current_point)?;
            // msg!("买入 DAMM-V2 wsol_amount_in: {}  token_amount_out: {}", wsol_amount_in as f64 / 1_000_000_000.0, token_amount_out);
            Ok(token_amount_out)
        }
        ParsedPoolState::PUMP { state } => {
            let token_amount_out = pump_quote_exact_input_wsol(state, wsol_mint, wsol_amount_in)?;
            // msg!("买入 Pump wsol_amount_in: {}  token_amount_out: {}", wsol_amount_in as f64 / 1_000_000_000.0, token_amount_out);
            Ok(token_amount_out)
        }
    }
}

/// 优化版卖出接口 - 使用预解析的bin数据
fn quote_sell_token_for_wsol_optimized(
    pool_state: &ParsedPoolState,
    wsol_mint: Pubkey,
    token_amount_in: u64,
    ordered_bins: &Option<Vec<(i32, crate::dex::dlmm::BinState)>>,
) -> Result<u64> {
    match pool_state {
        ParsedPoolState::CPMM { state } => {
            let wsol_amount_out = cpmm_quote_exact_input_token(state, wsol_mint, token_amount_in)?;
            // msg!("卖出 CPMM token_amount_in: {}  wsol_amount_out: {:.9} SOL", token_amount_in, wsol_amount_out as f64 / 1_000_000_000.0);
            Ok(wsol_amount_out)
        }
        ParsedPoolState::DLMM { state, .. } => {
            // 使用预解析的bin数据
            if let Some(bins) = ordered_bins {
                let wsol_amount_out = dlmm_quote_exact_input_token_optimized(
                    state,
                    wsol_mint,
                    token_amount_in,
                    bins,
                )?;

                //  msg!("卖出 DLMM token_amount_in: {}  wsol_amount_out: {:.9} SOL", token_amount_in, wsol_amount_out as f64 / 1_000_000_000.0);
                Ok(wsol_amount_out)
            } else {
                Err(ErrorCode::NoProfit.into())
            }
        }
        ParsedPoolState::DAMMV2 { state } => {
            let current_point = match state.activation_type {
                0 => Clock::get()?.slot,
                1 => Clock::get()?.unix_timestamp as u64,
                _ => return Err(ErrorCode::InvalidActivationType.into()),
            };

            let wsol_amount_out =
                dammv2_quote_exact_input_token(state, wsol_mint, token_amount_in, current_point)?;
            // msg!("卖出 DAMM-V2 token_amount_in: {}  wsol_amount_out: {:.9} SOL", token_amount_in, wsol_amount_out as f64 / 1_000_000_000.0);
            Ok(wsol_amount_out)
        }
        ParsedPoolState::PUMP { state } => {
            let wsol_amount_out = pump_quote_exact_input_token(state, wsol_mint, token_amount_in)?;

            // msg!("卖出 Pump token_amount_in: {}  wsol_amount_out: {:.9} SOL", token_amount_in, wsol_amount_out as f64 / 1_000_000_000.0);
            Ok(wsol_amount_out)
        }
    }
}
