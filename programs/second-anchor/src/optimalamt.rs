use crate::comparison::{MidPoolInfo, ParsedPoolState, TwoHopTokenInfo};
use crate::constant::WSOL_MINT;
use crate::dex::clmm::{
    calculate_clmm_amount_in_range, clmm_quote_exact_input_token,
    clmm_quote_exact_input_token_output_token, clmm_quote_exact_input_wsol,
    extract_tick_array_bitmap, parse_tick_array_states, ClmmParams, TickArrayBitmapExtension,
};
use crate::dex::cpmm::{
    cpmm_quote_exact_input_token_output_token, cpmm_quote_exact_input_token_output_wsol,
    cpmm_quote_exact_input_wsol,
};
use crate::dex::dammv2::{
    dammv2_quote_exact_input_token, dammv2_quote_exact_input_token1_output_token2,
    dammv2_quote_exact_input_wsol,
};
use crate::dex::dlmm::{
    dlmm_quote_exact_input_token_optimized, dlmm_quote_exact_input_token_output_token,
    dlmm_quote_exact_input_wsol_optimized, parse_dlmm_bin_arrays_optimized,
};
use crate::dex::pump::{
    pump_quote_exact_input_token, pump_quote_exact_input_token_output_token,
    pump_quote_exact_input_wsol,
};
use crate::dex::raydium::{raydium_quote_exact_input_token, raydium_quote_exact_input_wsol};
use crate::dex::whirlpool::{
    calculate_whirlpool_amount_in_range, parse_whirlpool_oracle_adaptive_fee,
    parse_whirlpool_tick_arrays_three, whirlpool_quote_exact_input_token,
    whirlpool_quote_exact_input_token_output_token, whirlpool_quote_exact_input_wsol,
    WhirlpoolParams,
};
use crate::utils::errors::ErrorCode;
use crate::utils::u128x128_math::{integer_sqrt_u128, safe_mul_div_cast, Rounding};
use crate::utils::u64x64_math::ONE;
 
use anchor_lang::prelude::*;
use std::cmp::min;

/// 提取token索引信息的辅助函数
#[inline]
fn extract_token_indices(
    mid_pool_info: Option<&MidPoolInfo>,
    two_hop_token_info: Option<&TwoHopTokenInfo>,
) -> (usize, usize, usize, Option<usize>, Option<usize>, Option<usize>, bool) {
    if let Some(mid_info) = mid_pool_info {
        // 3hop情况
        (
            mid_info.token1_mint_index,
            mid_info.token1_program_index,
            mid_info.token1_mint_token_account_index,
            Some(mid_info.token2_mint_index),
            Some(mid_info.token2_program_index),
            Some(mid_info.token2_mint_token_account_index),
            true,
        )
    } else if let Some(two_hop_info) = two_hop_token_info {
        // 2hop情况
        (
            two_hop_info.token_mint_index,
            two_hop_info.token_program_index,
            two_hop_info.mint_token_account_index,
            None,
            None,
            None,
            false,
        )
    } else {
        // 应该不会到这里，但提供默认值避免panic
        (0, 0, 0, None, None, None, false)
    }
}

/// 提取池子信息的辅助函数
#[inline]
fn extract_pool_info(pool_state: &ParsedPoolState) -> (bool, bool, bool, u128) {
    match pool_state {
        ParsedPoolState::DLMM { state, .. } => (false, false, true, state.price),
        ParsedPoolState::CLMM { state, .. } => (false, true, false, state.price),
        ParsedPoolState::WHIRLPOOL { state, .. } => (true, false, false, state.price),
        ParsedPoolState::CPMM { state } => (false, false, false, state.price),
        ParsedPoolState::DAMMV2 { state, .. } => (false, false, false, state.price),
        ParsedPoolState::PUMP { state, .. } => (false, false, false, state.price),
        ParsedPoolState::RAYDIUM { state, .. } => (false, false, false, state.price),
    }
}

/// 套利计算参数集合
pub struct ArbitrageParams<'info> {
    pub buy_ordered_bins: Option<Vec<(i32, crate::dex::dlmm::BinState)>>,
    pub mid_ordered_bins: Option<Vec<(i32, crate::dex::dlmm::BinState)>>,
    pub sell_ordered_bins: Option<Vec<(i32, crate::dex::dlmm::BinState)>>,
    pub buy_clmm_params: Option<ClmmParams>,
    pub mid_clmm_params: Option<ClmmParams>,
    pub sell_clmm_params: Option<ClmmParams>,
    pub buy_whirlpool_params: Option<WhirlpoolParams<'info>>,
    pub mid_whirlpool_params: Option<WhirlpoolParams<'info>>,
    pub sell_whirlpool_params: Option<WhirlpoolParams<'info>>,
}

/// 套利优化结果
pub struct OptimizationResult {
    pub optimal_wsol_amount: u64,
    pub total_wsol_out: u64,
    pub max_profit: u64,
    pub max_token1_amount_out: u64,
    pub max_token2_amount_out: u64,
    pub buy_pool_index: usize,
    pub mid_pool_index: Option<usize>,
    pub sell_pool_index: usize,
    // Token索引信息，兼容2hop和3hop
    pub token1_mint_index: usize,        // 第一个token的mint索引
    pub token1_program_index: usize,     // 第一个token的program索引  
    pub token1_account_index: usize,     // 第一个token的账户索引
    pub token2_mint_index: Option<usize>,        // 第二个token的mint索引(3hop时使用)
    pub token2_program_index: Option<usize>,     // 第二个token的program索引(3hop时使用)
    pub token2_account_index: Option<usize>,     // 第二个token的账户索引(3hop时使用)
    pub is_3hop: bool,                   // 是否为3hop套利
}

pub fn find_optimal_wsol_amount_golden_section<'info>(
    buy_pool_state: &ParsedPoolState,
    mid_pool_info: Option<&MidPoolInfo>,
    sell_pool_state: &ParsedPoolState,
    max_wsol_balance: u64,
    accounts: &'info [AccountInfo<'info>],
    max_profit_ratio: u128,
    min_profit: u32,
    two_hop_token_info: Option<&TwoHopTokenInfo>,
) -> Result<OptimizationResult> {
    let mut precision = 100_000; // 0.0001 SOL
    require!(max_wsol_balance >= precision, ErrorCode::NoProfit);

    // 使用辅助函数提取池子信息
    let (is_whirlpool_buy, is_clmm_buy, is_dlmm_buy, buy_pool_price) =
        extract_pool_info(buy_pool_state);
    let (is_whirlpool_sell, is_clmm_sell, is_dlmm_sell, sell_pool_price) =
        extract_pool_info(sell_pool_state);

    let (is_whirlpool_mid, is_clmm_mid, is_dlmm_mid, _mid_pool_price) =
        if let Some(mid_state) = mid_pool_info {
            extract_pool_info(mid_state.pool_state.as_ref())
        } else {
            // 2hop套利，没有中间池
            (false, false, false, 0)
        };

    let is_clmm_involved = is_clmm_buy || is_clmm_sell || is_clmm_mid;
    let is_dlmm_involved = is_dlmm_buy || is_dlmm_sell || is_dlmm_mid;
    let is_whirlpool_involved = is_whirlpool_buy || is_whirlpool_sell || is_whirlpool_mid;

    let (buy_ordered_bins, mid_ordered_bins, sell_ordered_bins) = if is_dlmm_involved {
        get_ordered_bins_for_pools(
            accounts,
            buy_pool_state,
            mid_pool_info,
            sell_pool_state,
            buy_pool_price,
            sell_pool_price,
        )?
    } else {
        (None, None, None)
    };

    let (buy_clmm_params, mid_clmm_params, sell_clmm_params) = if is_clmm_involved {
        get_clmm_params(
            accounts,
            buy_pool_state,
            mid_pool_info,
            sell_pool_state,
            buy_pool_price,
            sell_pool_price,
        )?
    } else {
        (None, None, None)
    };

    let (buy_whirlpool_params, mid_whirlpool_params, sell_whirlpool_params) =
        if is_whirlpool_involved {
            get_whirlpool_params(
                accounts,
                buy_pool_state,
                mid_pool_info,
                sell_pool_state,
                buy_pool_price,
                sell_pool_price,
            )?
        } else {
            (None, None, None)
        };

    let max_effective_input_buy_wsol = get_max_effective_input_wsol_from_buy(
        buy_pool_state,
        max_profit_ratio,
        &buy_ordered_bins,
        accounts,
        sell_pool_price,
    )?;

    let max_effective_input_sell_wsol = get_max_effective_input_wsol_form_sell(
        sell_pool_state,
        max_profit_ratio,
        &sell_ordered_bins,
        accounts,
        buy_pool_price,
    )?;

    // 创建套利参数结构体
    let arbitrage_params = ArbitrageParams {
        buy_ordered_bins,
        mid_ordered_bins,
        sell_ordered_bins,
        buy_clmm_params,
        mid_clmm_params,
        sell_clmm_params,
        buy_whirlpool_params,
        mid_whirlpool_params,
        sell_whirlpool_params,
    };

    let mut left = 0;
    let mut right = min(max_effective_input_buy_wsol, max_effective_input_sell_wsol) / 3; //狙击时候为了快速计算，onchain /2就够了或者不用/

    msg!(
        "D {} J {}",
        max_effective_input_buy_wsol as f64 / 1_000_000_000.0,
        max_effective_input_sell_wsol as f64 / 1_000_000_000.0
    );
    // msg!(
    //     "Buy: {:?}, Sell: {:?}",
    //     buy_pool_state.get_pool_type(),
    //     sell_pool_state.get_pool_type()
    // );
    // msg!(
    //     "Buy: {:?}, Sell: {:?}",
    //     accounts[buy_pool_state.get_pool_index()].key(),
    //     accounts[sell_pool_state.get_pool_index()].key()
    // );

    // 足够小的测试点
    let test_point = min(100_000, right);
    let (profit, token_amount_out, token2_amount_out) = calculate_arbitrage_profit_optimized(
        buy_pool_state,
        mid_pool_info,
        sell_pool_state,
        test_point,
        &arbitrage_params,
        accounts,
        two_hop_token_info,
    )?;

    //测试dex用
    // msg!("profit: {}, token_amount_out: {}", profit, token_amount_out);
    // return Ok(OptimizationResult {
    //     optimal_wsol_amount: test_point,
    //     max_profit: 1234 as u64,
    //     max_mint_amount_out: token_amount_out,
    //     total_wsol_out: test_point + min_profit as u64,
    // });

    require!(profit > 0, ErrorCode::NoProfit);
    if right <= 100_000 && profit > min_profit as i64 {
        let (token1_mint_index, token1_program_index, token1_account_index, 
             token2_mint_index, token2_program_index, token2_account_index, is_3hop) = 
            extract_token_indices(mid_pool_info, two_hop_token_info);
            
        return Ok(OptimizationResult {
            optimal_wsol_amount: test_point,
            max_profit: profit as u64,
            max_token1_amount_out: token_amount_out,
            max_token2_amount_out: token2_amount_out,
            total_wsol_out: test_point + min_profit as u64,
            buy_pool_index: buy_pool_state.get_pool_index(),
            mid_pool_index: mid_pool_info.map(|m| m.pool_state.get_pool_index()),
            sell_pool_index: sell_pool_state.get_pool_index(),
            token1_mint_index,
            token1_program_index,
            token1_account_index,
            token2_mint_index,
            token2_program_index,
            token2_account_index,
            is_3hop,
        });
    }

    // 代表有利润的订单 可计算
    let mut first_denom = 5;
    let mut max_profit = profit;
    let mut best_amount = test_point;
    let mut max_token1_amount_out = token_amount_out;
    let mut max_token2_amount_out = token2_amount_out;

    let test_point2 = right / 160;
    if test_point < test_point2 {
        let (profit2, token_amount_out2, token2_amount_out2) = calculate_arbitrage_profit_optimized(
            buy_pool_state,
            mid_pool_info,
            sell_pool_state,
            test_point2,
            &arbitrage_params,
            accounts,
            two_hop_token_info,
        )?;

        // 在右边
        if profit2 > profit {
            max_profit = profit2;
            best_amount = test_point2;
            max_token1_amount_out = token_amount_out2;
            max_token2_amount_out = token2_amount_out2;
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
    let (final_max_profit, final_best_amount, final_max_token1_amount_out, final_max_token2_amount_out) =
        binary_search_optimal_amount(
            buy_pool_state,
            mid_pool_info,
            sell_pool_state,
            &arbitrage_params,
            accounts,
            left,
            right,
            precision,
            first_denom,
            max_profit,
            best_amount,
            max_token1_amount_out,
            max_token2_amount_out,
            two_hop_token_info,
        )?;

    max_profit = final_max_profit;
    best_amount = final_best_amount;
    max_token1_amount_out = final_max_token1_amount_out;
    max_token2_amount_out = final_max_token2_amount_out;

    //helius sender
    require!(max_profit > min_profit as i64, ErrorCode::NoProfit);

    // best_amount = min(best_amount, max_wsol_balance); //这里不知道max_wsol_balance输出的profit 有bug
    // if best_amount > max_wsol_balance {
    //     best_amount = max_wsol_balance;
    //     if buy_pool_state.get_pool_type() == Some(PoolType::PUMP) {
    //         msg!("re calculate pump");
    //         //重新计算best_amount的输出 max_mint_amount_out
    //         (max_profit, max_mint_amount_out) = calculate_arbitrage_profit_optimized(
    //             buy_pool_state,
    //             mid_pool_info,
    //             sell_pool_state,
    //             best_amount * 97 / 100,
    //             &arbitrage_params,
    //             accounts,
    //             two_hop_token_info,
    //         )?;

    //         require!(max_profit > min_profit as i64, ErrorCode::NoProfit);
    //     }
    // }

    let (token1_mint_index, token1_program_index, token1_account_index, 
         token2_mint_index, token2_program_index, token2_account_index, is_3hop) = 
        extract_token_indices(mid_pool_info, two_hop_token_info);

    Ok(OptimizationResult {
        optimal_wsol_amount: best_amount,
        max_profit: max_profit as u64,
        max_token1_amount_out,
        max_token2_amount_out,
        total_wsol_out: best_amount + min_profit as u64,
        buy_pool_index: buy_pool_state.get_pool_index(),
        mid_pool_index: mid_pool_info.map(|m| m.pool_state.get_pool_index()),
        sell_pool_index: sell_pool_state.get_pool_index(),
        token1_mint_index,
        token1_program_index,
        token1_account_index,
        token2_mint_index,
        token2_program_index,
        token2_account_index,
        is_3hop,
    })
}

/// 二分法搜索最优WSOL数量
#[allow(clippy::too_many_arguments)]
fn binary_search_optimal_amount<'info>(
    buy_pool_state: &ParsedPoolState,
    mid_pool_state: Option<&MidPoolInfo>,
    sell_pool_state: &ParsedPoolState,
    arbitrage_params: &ArbitrageParams<'info>,
    accounts: &'info [AccountInfo<'info>],
    mut left: u64,
    mut right: u64,
    precision: u64,
    first_denom: u64,
    mut max_profit: i64,
    mut best_amount: u64,
    mut max_token1_amount_out: u64,
    mut max_token2_amount_out: u64,
    two_hop_token_info: Option<&TwoHopTokenInfo>,
) -> Result<(i64, u64, u64, u64)> {
    let mut iterations = 0u8;
    let max_loop = if mid_pool_state.is_some() { 3 } else { 5 };

    while right - left > precision && iterations < max_loop {
        let mid = if iterations == 0 {
            (left + right) / first_denom
        } else {
            (left + right) / 2
        };

        let (mid_profit, mid_token_out, mid_token2_out) = calculate_arbitrage_profit_optimized(
            buy_pool_state,
            mid_pool_state,
            sell_pool_state,
            mid,
            arbitrage_params,
            accounts,
            two_hop_token_info,
        )?;

        iterations += 1;

        if mid_profit > max_profit {
            max_profit = mid_profit;
            best_amount = mid;
            max_token1_amount_out = mid_token_out;
            max_token2_amount_out = mid_token2_out;
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

    Ok((max_profit, best_amount, max_token1_amount_out, max_token2_amount_out))
}

/// 使用已知的池价格来限制bin解析范围，大幅减少CU消耗
pub fn get_ordered_bins_for_pools(
    accounts: &[AccountInfo],
    buy_pool_state: &ParsedPoolState,
    mid_pool_state: Option<&MidPoolInfo>,
    sell_pool_state: &ParsedPoolState,
    buy_pool_price: u128,
    sell_pool_price: u128,
) -> Result<(
    Option<Vec<(i32, crate::dex::dlmm::BinState)>>,
    Option<Vec<(i32, crate::dex::dlmm::BinState)>>,
    Option<Vec<(i32, crate::dex::dlmm::BinState)>>,
)> {
    let buy_ordered_bins = match buy_pool_state {
        ParsedPoolState::DLMM { state, .. } => {
            //wsol -> token
            let bins = parse_dlmm_bin_arrays_optimized(
                state,
                accounts,
                state.active_id,
                !(state.token_y_mint == WSOL_MINT), // WSOL输入，Token输出
                buy_pool_price,
                sell_pool_price,
                true,
                mid_pool_state,
            )?;
            Some(bins)
        }
        ParsedPoolState::CPMM { .. } => None,
        ParsedPoolState::DAMMV2 { .. } => None,
        ParsedPoolState::PUMP { .. } => None,
        ParsedPoolState::RAYDIUM { .. } => None,
        ParsedPoolState::CLMM { .. } => None,
        ParsedPoolState::WHIRLPOOL { .. } => None,
    };

    let mid_ordered_bins = if let Some(mid_state) = mid_pool_state {
        match mid_state.pool_state.as_ref() {
            ParsedPoolState::DLMM { state, .. } => Some(parse_dlmm_bin_arrays_optimized(
                state,
                accounts,
                state.active_id,
                mid_state.is_mid_a2b,
                buy_pool_price,
                sell_pool_price,
                true,
                mid_pool_state,
            )?),
            ParsedPoolState::CPMM { .. } => None,
            ParsedPoolState::DAMMV2 { .. } => None,
            ParsedPoolState::PUMP { .. } => None,
            ParsedPoolState::RAYDIUM { .. } => None,
            ParsedPoolState::CLMM { .. } => None,
            ParsedPoolState::WHIRLPOOL { .. } => None,
        }
    } else {
        None
    };

    let sell_ordered_bins = match sell_pool_state {
        ParsedPoolState::DLMM { state } => {
            //token -> wsol
            let bins = parse_dlmm_bin_arrays_optimized(
                state,
                accounts,
                state.active_id,
                state.token_y_mint == WSOL_MINT, // Token输入，WSOL输出
                buy_pool_price,                  // 传入买入池价格用于范围限制
                sell_pool_price,                 // 传入卖出池价格
                false,
                mid_pool_state,
            )?;
            Some(bins)
        }
        ParsedPoolState::CPMM { .. } => None,
        ParsedPoolState::DAMMV2 { .. } => None,
        ParsedPoolState::PUMP { .. } => None,
        ParsedPoolState::RAYDIUM { .. } => None,
        ParsedPoolState::CLMM { .. } => None,
        ParsedPoolState::WHIRLPOOL { .. } => None,
    };

    Ok((buy_ordered_bins, mid_ordered_bins, sell_ordered_bins))
}

/// 避免在每次计算中重新解析bin arrays
fn calculate_arbitrage_profit_optimized<'info>(
    buy_pool_state: &ParsedPoolState,
    mid_pool_info: Option<&MidPoolInfo>,
    sell_pool_state: &ParsedPoolState,
    wsol_amount_in: u64,
    arbitrage_params: &ArbitrageParams<'_>,
    accounts: &'info [AccountInfo<'info>],
    two_hop_token_info: Option<&TwoHopTokenInfo>,
) -> Result<(i64, u64, u64)> {
    if wsol_amount_in == 0 {
        return Ok((0, 0, 0));
    }

    // 步骤1: 在买入池用SOL买Token
    // 获取token1的mint索引：3hop从mid_pool_info获取，2hop从two_hop_token_info获取
    let token1_mint_index = if let Some(mid_info) = mid_pool_info {
        mid_info.token1_mint_index
    } else if let Some(two_hop_info) = two_hop_token_info {
        two_hop_info.token_mint_index
    } else {
        return Err(ErrorCode::NoProfit.into()); // 没有token信息时无法进行套利
    };
    
    let token1_mint_info = &accounts[token1_mint_index];
    let mut token_amount_out = quote_buy_token_with_wsol_optimized(
        buy_pool_state,
        wsol_amount_in,
        &arbitrage_params.buy_ordered_bins,
        &arbitrage_params.buy_clmm_params,
        &arbitrage_params.buy_whirlpool_params,
        token1_mint_info,
    )?;

    if token_amount_out == 0 {
        return Ok((0, 0, 0));
    }

    // 步骤2: 在中间池用Token卖出换Token (仅3hop情况)
    let mut token2_amount_out = 0;
    if let Some(mid_pool_state) = mid_pool_info {
        // 检查是否为3hop（有实际的中间池）
        let token2_mint_info = &accounts[mid_pool_state.token2_mint_index];
        token2_amount_out = quote_mid_token1_for_token2_optimized(
            mid_pool_state,
            token_amount_out,
            &arbitrage_params.mid_ordered_bins,
            &arbitrage_params.mid_clmm_params,
            &arbitrage_params.mid_whirlpool_params,
            token1_mint_info,
            token2_mint_info,
        )?;

        if token2_amount_out == 0 {
            return Ok((0, 0, 0));
        }
        
        token_amount_out = token2_amount_out;
    }

    // 步骤3: 在卖出池用Token卖出换SOL
    // 对于2hop使用token1，对于3hop使用token2
    let sell_token_mint_info = if let Some(mid_pool_state) = mid_pool_info {
        if mid_pool_state.pool_state.get_pool_type().is_some() {
            &accounts[mid_pool_state.token2_mint_index] // 3hop: 卖出token2
        } else {
            token1_mint_info // 2hop: 卖出token1
        }
    } else {
        token1_mint_info // 2hop: 卖出token1
    };

    let wsol_amount_out = quote_sell_token_for_wsol_optimized(
        sell_pool_state,
        token_amount_out,
        &arbitrage_params.sell_ordered_bins,
        &arbitrage_params.sell_clmm_params,
        &arbitrage_params.sell_whirlpool_params,
        sell_token_mint_info,
    )?;

    // 计算净利润
    let profit = (wsol_amount_out as i64).saturating_sub(wsol_amount_in as i64);

    Ok((profit, token_amount_out, token2_amount_out))
}

/// 优化版买入接口 - 使用预解析的bin数据
fn quote_buy_token_with_wsol_optimized(
    pool_state: &ParsedPoolState,
    wsol_amount_in: u64,
    ordered_bins: &Option<Vec<(i32, crate::dex::dlmm::BinState)>>,
    buy_clmm_params: &Option<ClmmParams>,
    buy_whirlpool_params: &Option<WhirlpoolParams<'_>>,
    token_mint_info: &AccountInfo,
) -> Result<u64> {
    match pool_state {
        ParsedPoolState::CPMM { state } => {
            let token_amount_out =
                cpmm_quote_exact_input_wsol(state, wsol_amount_in, token_mint_info)?;
            // msg!("买入 CPMM wsol_amount_in: {}  token_amount_out: {}", wsol_amount_in as f64 / 1_000_000_000.0, token_amount_out);
            Ok(token_amount_out)
        }
        ParsedPoolState::DLMM { state, .. } => {
            // 使用预解析的bin数据
            if let Some(bins) = ordered_bins {
                let token_amount_out = dlmm_quote_exact_input_wsol_optimized(
                    state,
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
                wsol_amount_in,
                current_point,
                token_mint_info,
            )?;
            // msg!("买入 DAMM-V2 wsol_amount_in: {}  token_amount_out: {}", wsol_amount_in as f64 / 1_000_000_000.0, token_amount_out);
            Ok(token_amount_out)
        }
        ParsedPoolState::PUMP { state } => {
            let token_amount_out =
                pump_quote_exact_input_wsol(state, wsol_amount_in, token_mint_info)?;
            // msg!("买入 Pump wsol_amount_in: {}  token_amount_out: {}", wsol_amount_in as f64 / 1_000_000_000.0, token_amount_out);
            Ok(token_amount_out)
        }
        ParsedPoolState::RAYDIUM { state } => {
            let token_amount_out = raydium_quote_exact_input_wsol(state, wsol_amount_in)?;
            // msg!("买入 Raydium wsol_amount_in: {}  token_amount_out: {}", wsol_amount_in as f64 / 1_000_000_000.0, token_amount_out);
            Ok(token_amount_out)
        }
        ParsedPoolState::CLMM { state, .. } => {
            let token_amount_out = clmm_quote_exact_input_wsol(
                state,
                wsol_amount_in,
                token_mint_info,
                buy_clmm_params,
            )?;
            Ok(token_amount_out)
        }
        ParsedPoolState::WHIRLPOOL { state, .. } => {
            let token_amount_out = whirlpool_quote_exact_input_wsol(
                state,
                wsol_amount_in,
                token_mint_info,
                buy_whirlpool_params,
            )?;
            Ok(token_amount_out)
        }
    }
}

fn quote_mid_token1_for_token2_optimized(
    mid_pool_info: &MidPoolInfo,
    token_amount_in: u64,
    ordered_bins: &Option<Vec<(i32, crate::dex::dlmm::BinState)>>,
    mid_clmm_params: &Option<ClmmParams>,
    mid_whirlpool_params: &Option<WhirlpoolParams<'_>>,
    token1_mint_info: &AccountInfo,
    token2_mint_info: &AccountInfo,
) -> Result<u64> {
    match mid_pool_info.pool_state.as_ref() {
        ParsedPoolState::CPMM { state } => {
            let token2_amount_out = cpmm_quote_exact_input_token_output_token(
                state,
                token_amount_in,
                token1_mint_info,
                token2_mint_info,
            )?;
            // msg!("卖出 CPMM token_amount_in: {}  wsol_amount_out: {:.9} SOL", token_amount_in, wsol_amount_out as f64 / 1_000_000_000.0);
            Ok(token2_amount_out)
        }
        ParsedPoolState::DLMM { state, .. } => {
            // 使用预解析的bin数据
            if let Some(bins) = ordered_bins {
                let token2_amount_out = dlmm_quote_exact_input_token_output_token(
                    state,
                    mid_pool_info.is_mid_a2b,
                    token_amount_in,
                    bins,
                    token1_mint_info,
                    token2_mint_info,
                )?;

                //  msg!("卖出 DLMM token_amount_in: {}  wsol_amount_out: {:.9} SOL", token_amount_in, wsol_amount_out as f64 / 1_000_000_000.0);
                Ok(token2_amount_out)
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

            let token2_amount_out = dammv2_quote_exact_input_token1_output_token2(
                state,
                mid_pool_info.is_mid_a2b,
                token_amount_in,
                current_point,
                token1_mint_info,
                token2_mint_info,
            )?;
            // msg!("卖出 DAMM-V2 token_amount_in: {}  wsol_amount_out: {:.9} SOL", token_amount_in, wsol_amount_out as f64 / 1_000_000_000.0);
            Ok(token2_amount_out)
        }
        ParsedPoolState::PUMP { state } => {
            let token2_amount_out = pump_quote_exact_input_token_output_token(
                state,
                mid_pool_info.is_mid_a2b,
                token_amount_in,
                token1_mint_info,
                token2_mint_info,
            )?;

            // msg!("卖出 Pump token_amount_in: {}  wsol_amount_out: {:.9} SOL", token_amount_in, wsol_amount_out as f64 / 1_000_000_000.0);
            Ok(token2_amount_out)
        }
        ParsedPoolState::RAYDIUM { state } => {
            let token2_amount_out = raydium_quote_exact_input_token(state, token_amount_in)?;

            // msg!("卖出 Raydium token_amount_in: {}  wsol_amount_out: {:.9} SOL", token_amount_in, wsol_amount_out as f64 / 1_000_000_000.0);
            Ok(token2_amount_out)
        }
        ParsedPoolState::CLMM { state, .. } => {
            let token2_amount_out = clmm_quote_exact_input_token_output_token(
                state,
                mid_pool_info.is_mid_a2b,
                token_amount_in,
                token1_mint_info,
                token2_mint_info,
                mid_clmm_params,
            )?;
            Ok(token2_amount_out)
        }
        ParsedPoolState::WHIRLPOOL { state, .. } => {
            let token2_amount_out = whirlpool_quote_exact_input_token_output_token(
                state,
                mid_pool_info.is_mid_a2b,
                token_amount_in,
                token1_mint_info,
                token2_mint_info,
                mid_whirlpool_params,
            )?;
            Ok(token2_amount_out)
        }
    }
}

/// 优化版卖出接口 - 使用预解析的bin数据
fn quote_sell_token_for_wsol_optimized(
    pool_state: &ParsedPoolState,
    token_amount_in: u64,
    ordered_bins: &Option<Vec<(i32, crate::dex::dlmm::BinState)>>,
    sell_clmm_params: &Option<ClmmParams>,
    sell_whirlpool_params: &Option<WhirlpoolParams<'_>>,
    token_mint_info: &AccountInfo,
) -> Result<u64> {
    match pool_state {
        ParsedPoolState::CPMM { state } => {
            let wsol_amount_out =
                cpmm_quote_exact_input_token_output_wsol(state, token_amount_in, token_mint_info)?;
            // msg!("卖出 CPMM token_amount_in: {}  wsol_amount_out: {:.9} SOL", token_amount_in, wsol_amount_out as f64 / 1_000_000_000.0);
            Ok(wsol_amount_out)
        }
        ParsedPoolState::DLMM { state, .. } => {
            // 使用预解析的bin数据
            if let Some(bins) = ordered_bins {
                let wsol_amount_out = dlmm_quote_exact_input_token_optimized(
                    state,
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
                token_amount_in,
                current_point,
                token_mint_info,
            )?;
            // msg!("卖出 DAMM-V2 token_amount_in: {}  wsol_amount_out: {:.9} SOL", token_amount_in, wsol_amount_out as f64 / 1_000_000_000.0);
            Ok(wsol_amount_out)
        }
        ParsedPoolState::PUMP { state } => {
            let wsol_amount_out =
                pump_quote_exact_input_token(state, token_amount_in, token_mint_info)?;

            // msg!("卖出 Pump token_amount_in: {}  wsol_amount_out: {:.9} SOL", token_amount_in, wsol_amount_out as f64 / 1_000_000_000.0);
            Ok(wsol_amount_out)
        }
        ParsedPoolState::RAYDIUM { state } => {
            let wsol_amount_out = raydium_quote_exact_input_token(state, token_amount_in)?;

            // msg!("卖出 Raydium token_amount_in: {}  wsol_amount_out: {:.9} SOL", token_amount_in, wsol_amount_out as f64 / 1_000_000_000.0);
            Ok(wsol_amount_out)
        }
        ParsedPoolState::CLMM { state, .. } => {
            let wsol_amount_out = clmm_quote_exact_input_token(
                state,
                token_amount_in,
                token_mint_info,
                sell_clmm_params,
            )?;
            Ok(wsol_amount_out)
        }
        ParsedPoolState::WHIRLPOOL { state, .. } => {
            let wsol_amount_out = whirlpool_quote_exact_input_token(
                state,
                token_amount_in,
                token_mint_info,
                sell_whirlpool_params,
            )?;
            Ok(wsol_amount_out)
        }
    }
}

/// 获取CLMM参数
fn get_clmm_params<'c, 'info>(
    accounts: &'c [AccountInfo<'info>],
    buy_pool_state: &ParsedPoolState,
    mid_pool_state: Option<&MidPoolInfo>,
    sell_pool_state: &ParsedPoolState,
    buy_pool_price: u128,
    sell_pool_price: u128,
) -> Result<(Option<ClmmParams>, Option<ClmmParams>, Option<ClmmParams>)> {
    let buy_clmm_params: Option<ClmmParams> = match buy_pool_state {
        ParsedPoolState::CLMM { state } => {
            let sqrt_price_limit_x64 = if mid_pool_state.is_some() {
                0
            } else {
                let mut int_sqrt_price_limit_x64 = integer_sqrt_u128(sell_pool_price) << 32; // Q64.64格式的开平方

                // 确保sqrt价格格式一致
                if state.token_mint_1 != WSOL_MINT {
                    int_sqrt_price_limit_x64 =
                        safe_mul_div_cast(ONE, ONE, int_sqrt_price_limit_x64, Rounding::Up);
                }
                int_sqrt_price_limit_x64
            };

            let tick_arrays = vec![
                &accounts[state.tick_array_minus_1_index],
                &accounts[state.tick_array_0_index],
                &accounts[state.tick_array_1_index],
            ];
            let tick_array_states = parse_tick_array_states(&tick_arrays)?;
            let bitmap_extension_account = &accounts[state.bitmap_extension_index];
            // 提前解析bitmap extension
            let bitmap_extension =
                TickArrayBitmapExtension::parse_from_account_info(bitmap_extension_account)?;
            let tick_array_bitmap = extract_tick_array_bitmap(&accounts[state.pool_index])?;
            Some(ClmmParams {
                tick_array_states: tick_array_states,
                bitmap_extension, // 使用预解析的bitmap extension
                sqrt_price_limit_x64,
                tick_array_bitmap: tick_array_bitmap,
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
            let tick_array_states = parse_tick_array_states(&tick_arrays)?;
            let bitmap_extension_account = &accounts[state.bitmap_extension_index];
            // 提前解析bitmap extension
            let bitmap_extension =
                TickArrayBitmapExtension::parse_from_account_info(bitmap_extension_account)?;

            // 对于中间池，价格限制可以设为0（无限制）
            let sqrt_price_limit_x64 = if mid_pool_state.is_some() {
                0
            } else {
                let mut int_sqrt_price_limit_x64 = integer_sqrt_u128(buy_pool_price) << 32;
                // 确保sqrt价格格式一致
                if state.token_mint_1 != WSOL_MINT {
                    int_sqrt_price_limit_x64 =
                        safe_mul_div_cast(ONE, ONE, int_sqrt_price_limit_x64, Rounding::Down);
                }
                int_sqrt_price_limit_x64
            };

            let tick_array_bitmap = extract_tick_array_bitmap(&accounts[state.pool_index])?;
            Some(ClmmParams {
                tick_array_states: tick_array_states,
                bitmap_extension, // 使用预解析的bitmap extension
                sqrt_price_limit_x64,
                tick_array_bitmap: tick_array_bitmap,
            })
        }
        _ => None,
    };

    // 处理中间池
    let mid_clmm_params: Option<ClmmParams> = if let Some(mid_state) = mid_pool_state {
        match mid_state.pool_state.as_ref() {
            ParsedPoolState::CLMM { state } => {
                let tick_arrays = vec![
                    &accounts[state.tick_array_minus_1_index],
                    &accounts[state.tick_array_0_index],
                    &accounts[state.tick_array_1_index],
                ];
                let tick_array_states = parse_tick_array_states(&tick_arrays)?;

                let bitmap_extension_account = &accounts[state.bitmap_extension_index];
                let bitmap_extension =
                    TickArrayBitmapExtension::parse_from_account_info(bitmap_extension_account)?;

                // 对于中间池，价格限制可以设为0（无限制）
                let sqrt_price_limit_x64 = 0;
                let tick_array_bitmap = extract_tick_array_bitmap(&accounts[state.pool_index])?;

                Some(ClmmParams {
                    tick_array_states: tick_array_states,
                    bitmap_extension,
                    sqrt_price_limit_x64,
                    tick_array_bitmap: tick_array_bitmap,
                })
            }
            _ => None,
        }
    } else {
        None
    };

    Ok((buy_clmm_params, mid_clmm_params, sell_clmm_params))
}

/// 获取Whirlpool参数
fn get_whirlpool_params<'info>(
    accounts: &'info [AccountInfo<'info>],
    buy_pool_state: &ParsedPoolState,
    mid_pool_state: Option<&MidPoolInfo>,
    sell_pool_state: &ParsedPoolState,
    buy_pool_price: u128,
    sell_pool_price: u128,
) -> Result<(
    Option<WhirlpoolParams<'info>>,
    Option<WhirlpoolParams<'info>>,
    Option<WhirlpoolParams<'info>>,
)> {
    let buy_whirlpool_params: Option<WhirlpoolParams<'info>> = match buy_pool_state {
        ParsedPoolState::WHIRLPOOL { state } => {
            let sqrt_price_limit_x64 = if mid_pool_state.is_some() {
                0
            } else {
                let mut int_sqrt_price_limit_x64 = integer_sqrt_u128(sell_pool_price) << 32; // Q64.64格式的开平方
                if state.token_mint_b != WSOL_MINT {
                    int_sqrt_price_limit_x64 =
                        safe_mul_div_cast(ONE, ONE, int_sqrt_price_limit_x64, Rounding::Up);
                }
                int_sqrt_price_limit_x64
            };

            let tick_array_states = parse_whirlpool_tick_arrays_three(
                &accounts[state.tick_array_0_index],
                &accounts[state.tick_array_1_index],
                &accounts[state.tick_array_2_index],
            )?;
            let oracle_info = parse_whirlpool_oracle_adaptive_fee(&accounts[state.oracle_index])?;
            Some(WhirlpoolParams {
                tick_arrays: tick_array_states,
                sqrt_price_limit: sqrt_price_limit_x64,
                oracle_info: Box::new(oracle_info),
            })
        }
        _ => None,
    };
    let sell_whirlpool_params: Option<WhirlpoolParams<'info>> = match sell_pool_state {
        ParsedPoolState::WHIRLPOOL { state } => {
            let tick_array_states = parse_whirlpool_tick_arrays_three(
                &accounts[state.tick_array_0_index],
                &accounts[state.tick_array_1_index],
                &accounts[state.tick_array_2_index],
            )?;

            let sqrt_price_limit_x64 = if mid_pool_state.is_some() {
                0
            } else {
                let mut int_sqrt_price_limit_x64 = integer_sqrt_u128(buy_pool_price) << 32; // Q64.64格式的开平方
                if state.token_mint_b != WSOL_MINT {
                    int_sqrt_price_limit_x64 =
                        safe_mul_div_cast(ONE, ONE, int_sqrt_price_limit_x64, Rounding::Down);
                }
                int_sqrt_price_limit_x64
            };

            let oracle_info = parse_whirlpool_oracle_adaptive_fee(&accounts[state.oracle_index])?;
            Some(WhirlpoolParams {
                tick_arrays: tick_array_states,
                sqrt_price_limit: sqrt_price_limit_x64,
                oracle_info: Box::new(oracle_info),
            })
        }
        _ => None,
    };

    // 处理中间池
    let mid_whirlpool_params: Option<WhirlpoolParams<'info>> =
        if let Some(mid_state) = mid_pool_state {
            match mid_state.pool_state.as_ref() {
                ParsedPoolState::WHIRLPOOL { state } => {
                    // 对于中间池，价格限制可以设为0
                    let sqrt_price_limit_x64 = 0;

                    let oracle_info =
                        parse_whirlpool_oracle_adaptive_fee(&accounts[state.oracle_index])?;
                    Some(WhirlpoolParams {
                        tick_arrays: parse_whirlpool_tick_arrays_three(
                            &accounts[state.tick_array_0_index],
                            &accounts[state.tick_array_1_index],
                            &accounts[state.tick_array_2_index],
                        )?,
                        sqrt_price_limit: sqrt_price_limit_x64,
                        oracle_info: Box::new(oracle_info),
                    })
                }
                _ => None,
            }
        } else {
            None
        };

    Ok((
        buy_whirlpool_params,
        mid_whirlpool_params,
        sell_whirlpool_params,
    ))
}

fn get_max_effective_input_wsol_from_buy<'info>(
    buy_pool_state: &ParsedPoolState,
    max_profit_ratio: u128,
    buy_ordered_bins: &Option<Vec<(i32, crate::dex::dlmm::BinState)>>,
    accounts: &'info [AccountInfo<'info>],
    sell_pool_price: u128,
) -> Result<u64> {
    let max_effective_input_buy = match buy_pool_state {
        ParsedPoolState::CPMM { state } => {
            let wsol_reserve = if state.token0_mint == WSOL_MINT {
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
            let wsol_vault_index = if state.token_a_mint == WSOL_MINT {
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
            let wsol_reserve = if state.quote_mint == WSOL_MINT {
                state.quote_reserve
            } else {
                state.base_reserve
            };
            safe_mul_div_cast(wsol_reserve as u128, max_profit_ratio, ONE, Rounding::Down) as u64
            //后续版本优化算法
        }
        ParsedPoolState::RAYDIUM { state, .. } => {
            let wsol_reserve = if state.base_mint == WSOL_MINT {
                state.base_reserve
            } else {
                state.quote_reserve
            };
            safe_mul_div_cast(wsol_reserve as u128, max_profit_ratio, ONE, Rounding::Down) as u64
        }
        ParsedPoolState::CLMM { state, .. } => {
            let wsol_is_token_0 = state.token_mint_0 == WSOL_MINT;
            // 将Q64.64格式的价格转换为Q64.64格式的开平方
            // sqrt(price_q64) = sqrt(price * 2^64) = sqrt(price) * 2^32
            let mut sqrt_sell_price_x64 = integer_sqrt_u128(sell_pool_price) << 32;

            // 关键修复：确保sqrt价格格式一致
            // sell_pool_price是SOL/token格式，但state.sqrt_price_x64是原始的sqrt(token1/token0)格式
            // 如果WSOL不是token1，需要取sqrt_sell_price_x64的倒数来匹配原始格式
            if state.token_mint_1 != WSOL_MINT {
                // sqrt_sell_price_x64 现在是 sqrt(SOL/token) = sqrt(token0/token1)
                // 需要转换为 sqrt(token1/token0) 来匹配 state.sqrt_price_x64
                sqrt_sell_price_x64 =
                    safe_mul_div_cast(ONE, ONE, sqrt_sell_price_x64, Rounding::Down);
            }

            // 直接根据SOL的位置计算对应token的数量变化
            calculate_clmm_amount_in_range(
                state.sqrt_price_x64,
                sqrt_sell_price_x64,
                state.liquidity,
                wsol_is_token_0,
                false, // 计算输入量
            )?
            .unwrap_or(10)
        }
        ParsedPoolState::WHIRLPOOL { state, .. } => {
            let wsol_is_token_a = state.token_mint_a == WSOL_MINT;
            // 将Q64.64格式的价格转换为Q64.64格式的开平方
            // sqrt(price_q64) = sqrt(price * 2^64) = sqrt(price) * 2^32
            let mut sqrt_sell_price_x64 = integer_sqrt_u128(sell_pool_price) << 32;

            // 关键修复：确保sqrt价格格式一致
            // sell_pool_price是SOL/token格式，但state.sqrt_price是原始的sqrt(token_mint_b/token_mint_a)格式
            // 如果WSOL不是token_mint_b，需要取sqrt_sell_price_x64的倒数来匹配原始格式
            if state.token_mint_b != WSOL_MINT {
                // sqrt_sell_price_x64 现在是 sqrt(SOL/token) = sqrt(token_mint_a/token_mint_b)
                // 需要转换为 sqrt(token_mint_b/token_mint_a) 来匹配 state.sqrt_price
                sqrt_sell_price_x64 =
                    safe_mul_div_cast(ONE, ONE, sqrt_sell_price_x64, Rounding::Down);
            }

            calculate_whirlpool_amount_in_range(
                state.sqrt_price,
                sqrt_sell_price_x64,
                state.liquidity,
                wsol_is_token_a,
                false, // 计算输入量
            )?
            .unwrap_or(10)
        }
    };

    Ok(max_effective_input_buy)
}

fn get_max_effective_input_wsol_form_sell<'info>(
    sell_pool_state: &ParsedPoolState,
    max_profit_ratio: u128,
    sell_ordered_bins: &Option<Vec<(i32, crate::dex::dlmm::BinState)>>,
    accounts: &'info [AccountInfo<'info>],
    buy_pool_price: u128,
) -> Result<u64> {
    let max_effective_input_sell = match sell_pool_state {
        ParsedPoolState::CPMM { state } => {
            let wsol_reserve = if state.token0_mint == WSOL_MINT {
                state.token0_reserve
            } else {
                state.token1_reserve
            };

            safe_mul_div_cast(wsol_reserve as u128, max_profit_ratio, ONE, Rounding::Down) as u64
        }
        ParsedPoolState::DLMM { state, .. } => {
            if let Some(ref bins) = sell_ordered_bins {
                if state.token_y_mint == WSOL_MINT {
                    bins.iter().map(|(_, bin)| bin.amount_y).sum::<u64>()
                } else {
                    bins.iter().map(|(_, bin)| bin.amount_x).sum::<u64>()
                }
            } else {
                0
            }
        }
        ParsedPoolState::DAMMV2 { state, .. } => {
            let wsol_vault_index = if state.token_a_mint == WSOL_MINT {
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
            let wsol_reserve = if state.quote_mint == WSOL_MINT {
                state.quote_reserve
            } else {
                state.base_reserve
            };
            safe_mul_div_cast(wsol_reserve as u128, max_profit_ratio, ONE, Rounding::Down) as u64
        }
        ParsedPoolState::RAYDIUM { state, .. } => {
            let wsol_reserve = if state.base_mint == WSOL_MINT {
                state.base_reserve
            } else {
                state.quote_reserve
            };
            safe_mul_div_cast(wsol_reserve as u128, max_profit_ratio, ONE, Rounding::Down) as u64
        }
        ParsedPoolState::CLMM { state, .. } => {
            let wsol_is_token_0 = state.token_mint_0 == WSOL_MINT;
            let mut sqrt_buy_price_x64 = integer_sqrt_u128(buy_pool_price) << 32; // Q64.64格式的开平方

            // 确保sqrt价格格式一致
            if state.token_mint_1 != WSOL_MINT {
                sqrt_buy_price_x64 =
                    safe_mul_div_cast(ONE, ONE, sqrt_buy_price_x64, Rounding::Down);
            }

            // 直接根据SOL的位置计算对应token的数量变化
            calculate_clmm_amount_in_range(
                state.sqrt_price_x64,
                sqrt_buy_price_x64,
                state.liquidity,
                wsol_is_token_0,
                false, // 计算输入量
            )?
            .unwrap_or(10)
        }
        ParsedPoolState::WHIRLPOOL { state, .. } => {
            let wsol_is_token_a = state.token_mint_a == WSOL_MINT;
            let mut sqrt_buy_price_x64 = integer_sqrt_u128(buy_pool_price) << 32; // Q64.64格式的开平方

            // 确保sqrt价格格式一致
            if state.token_mint_b != WSOL_MINT {
                sqrt_buy_price_x64 =
                    safe_mul_div_cast(ONE, ONE, sqrt_buy_price_x64, Rounding::Down);
            }

            calculate_whirlpool_amount_in_range(
                state.sqrt_price,
                sqrt_buy_price_x64,
                state.liquidity,
                wsol_is_token_a,
                false, // 计算输入量
            )?
            .unwrap_or(10)
        }
    };

    Ok(max_effective_input_sell)
}
