use crate::comparison::ParsedPoolState;
use crate::dex::clmm::{clmm_quote_exact_input_token, clmm_quote_exact_input_wsol, calculate_amount_in_range, ClmmParams};
use crate::dex::cpmm::{cpmm_quote_exact_input_token, cpmm_quote_exact_input_wsol};
use crate::dex::dammv2::{dammv2_quote_exact_input_token, dammv2_quote_exact_input_wsol};
use crate::dex::dlmm::{
    dlmm_quote_exact_input_token_optimized, dlmm_quote_exact_input_wsol_optimized,
    parse_dlmm_bin_arrays_optimized,
};
use crate::dex::pump::{pump_quote_exact_input_token, pump_quote_exact_input_wsol};
use crate::dex::raydium::{raydium_quote_exact_input_token, raydium_quote_exact_input_wsol};
use crate::utils::errors::ErrorCode;
use crate::utils::u128x128_math::{integer_sqrt_u128, safe_mul_div_cast, Rounding};
use crate::utils::u64x64_math::ONE;
use anchor_lang::prelude::*;
use std::cmp::min;

/// 套利优化结果
#[derive(Debug)]
pub struct OptimizationResult {
    pub optimal_wsol_amount: u64,
    pub total_wsol_out: u64,
    pub max_profit: u64,
    pub max_mint_amount_out: u64,
}

/// 优化的黄金分割法寻找最优买入SOL数量
/// 基于套利利润函数单峰性质的智能优化算法
pub fn find_optimal_wsol_amount_golden_section<'c, 'info>(
    buy_pool_state: &ParsedPoolState,
    sell_pool_state: &ParsedPoolState,
    wsol_mint: Pubkey,
    max_wsol_balance: u64,
    accounts: &'c [AccountInfo<'info>],
    max_profit_ratio: u128,
    min_profit: u32,
    token_mint_info: &AccountInfo,
) -> Result<OptimizationResult> {
    let mut precision = 100_000; // 0.0001 SOL
    require!(max_wsol_balance >= precision, ErrorCode::NoProfit);

    // 首先提取池子价格用于后续计算
    let (is_clmm_buy, is_dlmm_buy, buy_pool_price) = match buy_pool_state {
        ParsedPoolState::CPMM { state } => (false, false, state.price),
        ParsedPoolState::DLMM { state, .. } => (false, true, state.price),
        ParsedPoolState::DAMMV2 { state, .. } => (false, false, state.price),
        ParsedPoolState::PUMP { state, .. } => (false, false, state.price),
        ParsedPoolState::RAYDIUM { state, .. } => (false, false, state.price),
        ParsedPoolState::CLMM { state, .. } => (true, false, state.price),
    };

    let (is_clmm_sell, is_dlmm_sell, sell_pool_price) = match sell_pool_state {
        ParsedPoolState::CPMM { state } => (false, false, state.price),
        ParsedPoolState::DLMM { state, .. } => (false, true, state.price),
        ParsedPoolState::DAMMV2 { state, .. } => (false, false, state.price),
        ParsedPoolState::PUMP { state, .. } => (false, false, state.price),
        ParsedPoolState::RAYDIUM { state, .. } => (false, false, state.price),
        ParsedPoolState::CLMM { state, .. } => (true, false, state.price),
    };

    let is_clmm_involved = is_clmm_buy || is_clmm_sell;
    let is_dlmm_involved = is_dlmm_buy || is_dlmm_sell;

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

    let (buy_clmm_params, sell_clmm_params) = if is_clmm_involved {
        get_clmm_params(
            accounts,
            buy_pool_state,
            sell_pool_state,
            buy_pool_price,
            sell_pool_price,
            wsol_mint,
        )?
    } else {
        (None, None)
    };

    let max_effective_input_buy_wsol = get_max_effective_input_wsol_from_buy(
        buy_pool_state,
        wsol_mint,
        max_profit_ratio,
        &buy_ordered_bins,
        accounts,
        sell_pool_price,
    )?;

    let max_effective_input_sell_wsol = get_max_effective_input_wsol_form_sell(
        sell_pool_state,
        wsol_mint,
        max_profit_ratio,
        &sell_ordered_bins,
        accounts,
        buy_pool_price,
    )?;

    let mut left = 0;
    let mut right = min(max_effective_input_buy_wsol, max_effective_input_sell_wsol);

    msg!(
        "D {} J {}",
        max_effective_input_buy_wsol as f64 / 1_000_000_000.0,
        max_effective_input_sell_wsol as f64 / 1_000_000_000.0
    );

    // 足够小的测试点
    let test_point = min(100_000, right);
    let (profit, token_amount_out) = calculate_arbitrage_profit_optimized(
        buy_pool_state,
        sell_pool_state,
        wsol_mint,
        test_point,
        &buy_ordered_bins,
        &sell_ordered_bins,
        &buy_clmm_params,
        &sell_clmm_params,
        token_mint_info,
    )?;

    // msg!("profit: {}, token_amount_out: {}", profit, token_amount_out);
    // return Ok(OptimizationResult {
    //     optimal_wsol_amount: test_point,
    //     max_profit: 1234,
    //     max_mint_amount_out: token_amount_out,
    //     total_wsol_out: test_point + 1234 as u64,
    // });
    
    require!(profit > 0, ErrorCode::NoProfit);
    if right <= 100_000 && profit > min_profit as i64 {
        return Ok(OptimizationResult {
            optimal_wsol_amount: test_point,
            max_profit: profit as u64,
            max_mint_amount_out: token_amount_out,
            total_wsol_out: test_point + min_profit as u64,
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
            &buy_clmm_params,
            &sell_clmm_params,
            token_mint_info,
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
            &buy_clmm_params,
            &sell_clmm_params,
            token_mint_info,
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

    best_amount = min(best_amount, max_wsol_balance); //这里不知道max_wsol_balance输出的profit 有bug

    Ok(OptimizationResult {
        optimal_wsol_amount: best_amount,
        max_profit: max_profit as u64,
        max_mint_amount_out,
        total_wsol_out: best_amount + min_profit as u64,
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
        ParsedPoolState::RAYDIUM { .. } => None,
        ParsedPoolState::CLMM { .. } => None,
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
        ParsedPoolState::RAYDIUM { .. } => None,
        ParsedPoolState::CLMM { .. } => None,
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
    buy_clmm_params: &Option<ClmmParams>,
    sell_clmm_params: &Option<ClmmParams>,
    token_mint_info: &AccountInfo,
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
        buy_clmm_params,
        token_mint_info,
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
        sell_clmm_params,
        token_mint_info,
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
    buy_clmm_params: &Option<ClmmParams>,
    token_mint_info: &AccountInfo,
) -> Result<u64> {
    match pool_state {
        ParsedPoolState::CPMM { state } => {
            let token_amount_out =
                cpmm_quote_exact_input_wsol(state, wsol_mint, wsol_amount_in, token_mint_info)?;
            // msg!("买入 CPMM wsol_amount_in: {}  token_amount_out: {}", wsol_amount_in as f64 / 1_000_000_000.0, token_amount_out);
            Ok(token_amount_out)
        }
        ParsedPoolState::DLMM { state, .. } => {
            // 使用预解析的bin数据
            if let Some(bins) = ordered_bins {
                let token_amount_out = dlmm_quote_exact_input_wsol_optimized(
                    state,
                    wsol_mint,
                    wsol_amount_in,
                    bins,
                    token_mint_info,
                )?;
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

            let token_amount_out = dammv2_quote_exact_input_wsol(
                state,
                wsol_mint,
                wsol_amount_in,
                current_point,
                token_mint_info,
            )?;
            // msg!("买入 DAMM-V2 wsol_amount_in: {}  token_amount_out: {}", wsol_amount_in as f64 / 1_000_000_000.0, token_amount_out);
            Ok(token_amount_out)
        }
        ParsedPoolState::PUMP { state } => {
            let token_amount_out =
                pump_quote_exact_input_wsol(state, wsol_mint, wsol_amount_in, token_mint_info)?;
            // msg!("买入 Pump wsol_amount_in: {}  token_amount_out: {}", wsol_amount_in as f64 / 1_000_000_000.0, token_amount_out);
            Ok(token_amount_out)
        }
        ParsedPoolState::RAYDIUM { state } => {
            let token_amount_out =
                raydium_quote_exact_input_wsol(state, wsol_mint, wsol_amount_in)?;
            // msg!("买入 Raydium wsol_amount_in: {}  token_amount_out: {}", wsol_amount_in as f64 / 1_000_000_000.0, token_amount_out);
            Ok(token_amount_out)
        }
        ParsedPoolState::CLMM { state, .. } => {
            let token_amount_out = clmm_quote_exact_input_wsol(
                state,
                wsol_mint,
                wsol_amount_in,
                token_mint_info,
                &buy_clmm_params,
            )?;
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
    sell_clmm_params: &Option<ClmmParams>,
    token_mint_info: &AccountInfo,
) -> Result<u64> {
    match pool_state {
        ParsedPoolState::CPMM { state } => {
            let wsol_amount_out =
                cpmm_quote_exact_input_token(state, wsol_mint, token_amount_in, token_mint_info)?;
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
                    token_mint_info,
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

            let wsol_amount_out = dammv2_quote_exact_input_token(
                state,
                wsol_mint,
                token_amount_in,
                current_point,
                token_mint_info,
            )?;
            // msg!("卖出 DAMM-V2 token_amount_in: {}  wsol_amount_out: {:.9} SOL", token_amount_in, wsol_amount_out as f64 / 1_000_000_000.0);
            Ok(wsol_amount_out)
        }
        ParsedPoolState::PUMP { state } => {
            let wsol_amount_out =
                pump_quote_exact_input_token(state, wsol_mint, token_amount_in, token_mint_info)?;

            // msg!("卖出 Pump token_amount_in: {}  wsol_amount_out: {:.9} SOL", token_amount_in, wsol_amount_out as f64 / 1_000_000_000.0);
            Ok(wsol_amount_out)
        }
        ParsedPoolState::RAYDIUM { state } => {
            let wsol_amount_out =
                raydium_quote_exact_input_token(state, wsol_mint, token_amount_in)?;

            // msg!("卖出 Raydium token_amount_in: {}  wsol_amount_out: {:.9} SOL", token_amount_in, wsol_amount_out as f64 / 1_000_000_000.0);
            Ok(wsol_amount_out)
        }
        ParsedPoolState::CLMM { state, .. } => {
            let wsol_amount_out = clmm_quote_exact_input_token(
                state,
                wsol_mint,
                token_amount_in,
                token_mint_info,
                &sell_clmm_params,
            )?;
            Ok(wsol_amount_out)
        }
    }
}

/// 获取CLMM参数
fn get_clmm_params<'c, 'info>(
    accounts: &'c [AccountInfo<'info>],
    buy_pool_state: &ParsedPoolState,
    sell_pool_state: &ParsedPoolState,
    buy_pool_price: u128,
    sell_pool_price: u128,
    wsol_mint: Pubkey,
) -> Result<(Option<ClmmParams<'c, 'info>>, Option<ClmmParams<'c, 'info>>)> {
    let buy_clmm_params: Option<ClmmParams> = match buy_pool_state {
        ParsedPoolState::CLMM { state } => {
            let mut sqrt_price_limit_x64 = integer_sqrt_u128(sell_pool_price) << 32; // Q64.64格式的开平方
            
            // 确保sqrt价格格式一致
            if state.token_mint_1 != wsol_mint {
                sqrt_price_limit_x64 = safe_mul_div_cast(ONE, ONE, sqrt_price_limit_x64, Rounding::Down);
            }
            
            let tick_arrays = vec![
                &accounts[state.tick_array_minus_1_index],
                &accounts[state.tick_array_0_index],
                &accounts[state.tick_array_1_index],
            ];
            let bitmap_extension = &accounts[state.bitmap_extension_index];
            Some(ClmmParams {
                tick_arrays,
                bitmap_extension,
                sqrt_price_limit_x64,
            })
        }
        _ => None,
    };
    let sell_clmm_params: Option<ClmmParams> = match sell_pool_state {
        ParsedPoolState::CLMM { state } => {
            let tick_arrays = vec![
                &accounts[state.tick_array_minus_1_index],
                &accounts[state.tick_array_0_index],
                &accounts[state.tick_array_1_index],
            ];
            let bitmap_extension = &accounts[state.bitmap_extension_index];
            let mut sqrt_price_limit_x64 = integer_sqrt_u128(buy_pool_price) << 32; // Q64.64格式的开平方
            
            // 确保sqrt价格格式一致
            if state.token_mint_1 != wsol_mint {
                sqrt_price_limit_x64 = safe_mul_div_cast(ONE, ONE, sqrt_price_limit_x64, Rounding::Down);
            }
            
            Some(ClmmParams {
                tick_arrays,
                bitmap_extension,
                sqrt_price_limit_x64,
            })
        }
        _ => None,
    };

    Ok((buy_clmm_params, sell_clmm_params))
}

fn get_max_effective_input_wsol_from_buy<'c, 'info>(
    buy_pool_state: &ParsedPoolState,
    wsol_mint: Pubkey,
    max_profit_ratio: u128,
    buy_ordered_bins: &Option<Vec<(i32, crate::dex::dlmm::BinState)>>,
    accounts: &'c [AccountInfo<'info>],
    sell_pool_price: u128,
) -> Result<u64> {
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
            let wsol_balance = {
                let wsol_vault_data = &accounts[wsol_vault_index].data.borrow();
                u64::from_le_bytes(wsol_vault_data[64..72].try_into().unwrap_or([0; 8]))
            };
        

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
        ParsedPoolState::RAYDIUM { state, .. } => {
            let wsol_reserve = if state.base_mint == wsol_mint {
                state.base_reserve
            } else {
                state.quote_reserve
            };
            safe_mul_div_cast(wsol_reserve as u128, max_profit_ratio, ONE, Rounding::Down) as u64
        }
        ParsedPoolState::CLMM { state, .. } => {
            let zero_for_one = if state.token_mint_0 == wsol_mint {
                true
            } else {
                false
            };
            // 将Q64.64格式的价格转换为Q64.64格式的开平方
            // sqrt(price_q64) = sqrt(price * 2^64) = sqrt(price) * 2^32
            let mut sqrt_sell_price_x64 = integer_sqrt_u128(sell_pool_price) << 32;
            
            // 关键修复：确保sqrt价格格式一致
            // sell_pool_price是SOL/token格式，但state.sqrt_price_x64是原始的sqrt(token1/token0)格式
            // 如果WSOL不是token1，需要取sqrt_sell_price_x64的倒数来匹配原始格式
            if state.token_mint_1 != wsol_mint {
                // sqrt_sell_price_x64 现在是 sqrt(SOL/token) = sqrt(token0/token1)
                // 需要转换为 sqrt(token1/token0) 来匹配 state.sqrt_price_x64
                sqrt_sell_price_x64 = safe_mul_div_cast(ONE, ONE, sqrt_sell_price_x64, Rounding::Down);
            }
            
            calculate_amount_in_range(
                state.sqrt_price_x64,
                sqrt_sell_price_x64,
                state.liquidity,
                zero_for_one,
                true,
            )?
            .unwrap_or(0)
        },
    };

    Ok(max_effective_input_buy)
}

fn get_max_effective_input_wsol_form_sell<'c, 'info>(
    sell_pool_state: &ParsedPoolState,
    wsol_mint: Pubkey,
    max_profit_ratio: u128,
    sell_ordered_bins: &Option<Vec<(i32, crate::dex::dlmm::BinState)>>,
    accounts: &'c [AccountInfo<'info>],
    buy_pool_price: u128,
) -> Result<u64> {
    let max_effective_input_sell = match sell_pool_state {
        ParsedPoolState::CPMM { state } => {
            let wsol_reserve = if state.token0_mint == wsol_mint {
                state.token0_reserve
            } else {
                state.token1_reserve
            };

            safe_mul_div_cast(wsol_reserve as u128, max_profit_ratio, ONE, Rounding::Down) as u64
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
            let wsol_balance = {
                let wsol_vault_data = &accounts[wsol_vault_index].data.borrow();
                u64::from_le_bytes(wsol_vault_data[64..72].try_into().unwrap_or([0; 8]))
            };
           
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
        }
        ParsedPoolState::RAYDIUM { state, .. } => {
            let wsol_reserve = if state.base_mint == wsol_mint {
                state.base_reserve
            } else {
                state.quote_reserve
            };
            safe_mul_div_cast(wsol_reserve as u128, max_profit_ratio, ONE, Rounding::Down) as u64
        }
        ParsedPoolState::CLMM { state, .. } => {
            let zero_for_one = if state.token_mint_0 == wsol_mint {
                false
            } else {
                true
            };
            let mut sqrt_buy_price_x64 = integer_sqrt_u128(buy_pool_price) << 32; // Q64.64格式的开平方
            
            // 确保sqrt价格格式一致
            if state.token_mint_1 != wsol_mint {
                sqrt_buy_price_x64 = safe_mul_div_cast(ONE, ONE, sqrt_buy_price_x64, Rounding::Down);
            }
            
            calculate_amount_in_range(
                state.sqrt_price_x64,
                sqrt_buy_price_x64,
                state.liquidity,
                zero_for_one,
                true,
            )?
            .unwrap_or(0)
        },
    };

    Ok(max_effective_input_sell)
}
