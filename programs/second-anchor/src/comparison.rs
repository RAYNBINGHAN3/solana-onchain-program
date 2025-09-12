use crate::dex::clmm::{calculate_clmm_price, parse_clmm_pool_data, ClmmPoolState};
use crate::dex::cpmm::{calculate_cpmm_price, parse_cpmm_pool_data, CpmmPoolState};
use crate::dex::dammv2::{
    calculate_dammv2_price, calculate_dammv2_total_fee_rate, parse_dammv2_pool_data,
    Dammv2PoolState,
};
use crate::dex::dlmm::{
    calculate_dlmm_total_fee_rate, get_dlmm_price_from_bin_array, parse_dlmm_pool_data,
    DlmmPoolState,
};
use crate::dex::pump::{calculate_pump_price, parse_pump_pool_data, PumpPoolState};
use crate::dex::raydium::{calculate_raydium_price, parse_raydium_pool_data, RaydiumPoolState};
use crate::dex::whirlpool::{parse_whirlpool_pool_data, WhirlpoolPoolState};
use crate::utils::errors::ErrorCode;
use crate::utils::u128x128_math::{bps_to_q64, q64_mul, safe_mul_div_cast, Rounding};
use crate::utils::u64x64_math::ONE;
use crate::utils::utils::{PoolData, PoolType, TokenPoolGroup};
use anchor_lang::prelude::*;

/// 解析好的池状态数据，避免重复解析
#[derive(Debug, Clone)]
pub enum ParsedPoolState {
    CPMM {
        state: CpmmPoolState,
    },
    DLMM {
        state: DlmmPoolState,
        reserve_x_index: usize,
        reserve_y_index: usize,
        bin_array_minus_1_index: usize,
        bin_array_0_index: usize,
        bin_array_1_index: usize,
    },
    DAMMV2 {
        state: Dammv2PoolState,
    },
    PUMP {
        state: PumpPoolState,
    },
    RAYDIUM {
        state: RaydiumPoolState,
    },
    CLMM {
        state: ClmmPoolState,
    },
    WHIRLPOOL {
        state: WhirlpoolPoolState,
    },
}

impl ParsedPoolState {
    pub fn get_pool_type(&self) -> Option<PoolType> {
        match self {
            ParsedPoolState::CPMM { .. } => Some(PoolType::CPMM),
            ParsedPoolState::DLMM { .. } => Some(PoolType::DLMM),
            ParsedPoolState::DAMMV2 { .. } => Some(PoolType::DAMMV2),
            ParsedPoolState::PUMP { .. } => Some(PoolType::PUMP),
            ParsedPoolState::RAYDIUM { .. } => Some(PoolType::RAYDIUM),
            ParsedPoolState::CLMM { .. } => Some(PoolType::CLMM),
            ParsedPoolState::WHIRLPOOL { .. } => Some(PoolType::WHIRLPOOL),
        }
    }
}

/// 全局套利分析结果 (包含解析好的数据，避免重复解析)
#[derive(Debug)]
pub struct GlobalArbitrageAnalysis {
    // pub has_opportunity: bool,
    pub best_token_mint_index: Option<usize>,
    pub max_profit_ratio: u128, // Q64.64格式利润率
    // 包含解析好的池状态数据 - 使用Box减少栈使用
    pub buy_pool_state: Option<Box<ParsedPoolState>>,
    pub sell_pool_state: Option<Box<ParsedPoolState>>,
    pub token_program_index: Option<usize>,
    pub mint_token_account_index: Option<usize>,
}

/// 分析多token的全局套利机会
pub fn analyze_global_arbitrage_opportunities(
    token_groups: &[TokenPoolGroup],
    wsol_mint: Pubkey,
    accounts: &[AccountInfo],
    is_dir_swap: bool,
) -> Result<GlobalArbitrageAnalysis> {
    let mut global_max_profit = 0u128; // Q64.64格式
    let mut best_token_mint_index = None;
    let mut best_token_program_index = None;
    let mut best_mint_token_account_index = None;
    let mut best_buy_pool_state = None;
    let mut best_sell_pool_state = None;
 

    // 遍历每个token组
    for token_group in token_groups {
        // 计算该token所有池的价格，同时解析池状态 - 使用Box减少栈使用
        let mut parsed_pools: Vec<Box<ParsedPoolState>> = Vec::with_capacity(6); //注意这里限制了一个mint最多6个池子

        for pool_data in &token_group.pools {
            match parse_pool_with_state(
                pool_data,
                token_group.token_mint_index,
                wsol_mint,
                accounts,
                is_dir_swap,
            ) {
                Ok(parsed_state) => {
                    // 转换Q64.64为f64用于打印
                    // let (price_f64, fee_rate, pool_type, pool_address) = match &parsed_state {
                    // ParsedPoolState::CPMM { state, .. } => (
                    //     state.price as f64 / ONE as f64,
                    //     state.total_fee_rate,
                    //     PoolType::CPMM,
                    //     accounts[state.pool_index].key,
                    // ),
                    // ParsedPoolState::DLMM { state, .. } => (
                    //     state.price as f64 / ONE as f64,
                    //     state.total_fee_rate / 1_000,
                    //     PoolType::DLMM,
                    //     accounts[state.pool_index].key,
                    // ),
                    // ParsedPoolState::DAMMV2 { state, .. } => (
                    //     state.price as f64 / ONE as f64,
                    //     state.total_fee_rate / 1_000,
                    //     PoolType::DAMMV2,
                    //     accounts[state.pool_index].key,
                    // ),
                    // ParsedPoolState::PUMP { state, .. } => (
                    //     state.price as f64 / ONE as f64,
                    //     state.trade_fee_rate * 100, //pump是1w bps, 所以需要*100
                    //     PoolType::PUMP,
                    //     accounts[state.pool_index].key,
                    // ),
                    // ParsedPoolState::CLMM { state, .. } => (
                    //     state.price as f64 / ONE as f64,
                    //     state.trade_fee_rate ,
                    //     PoolType::CLMM,
                    //     accounts[state.pool_index].key,
                    // ),
                    // ParsedPoolState::RAYDIUM { state, .. } => (
                    //     state.price as f64 / ONE as f64,
                    //     state.trade_fee_rate ,
                    //     PoolType::RAYDIUM,
                    //     accounts[state.pool_index].key,
                    // ),
                    // ParsedPoolState::WHIRLPOOL { state, .. } => (
                    //     state.price as f64 / ONE as f64,
                    //     state.trade_fee_rate ,
                    //     PoolType::WHIRLPOOL,
                    //     accounts[state.pool_index].key,
                    // ),
                    // };
                    // msg!(
                    //     " ==> Type: {:?}, Pool: {:?}, Price: {:.12} SOL, fee: {} bps",
                    //     pool_type,
                    //     pool_address,
                    //     price_f64,
                    //     fee_rate
                    // );
                    parsed_pools.push(Box::new(parsed_state));
                }
                Err(_) => {
                    continue;
                    // return Err(e.into()); // 其他错误，继续抛出
                }
            }
        }

        // 直接返回 不用对比价格
        if is_dir_swap {
            // 检查是否有足够的池子
            if parsed_pools.len() < 2 {
                continue; // 跳过这个token组，继续处理下一个
            }
            return Ok(GlobalArbitrageAnalysis {
                best_token_mint_index: Some(token_group.token_mint_index),
                max_profit_ratio: ONE,
                buy_pool_state: Some(parsed_pools[0].clone()),
                sell_pool_state: Some(parsed_pools[1].clone()),
                token_program_index: Some(token_group.token_program_index),
                mint_token_account_index: Some(token_group.mint_token_account_index),
            });
        }

        // 分析该token的套利机会
        if parsed_pools.len() >= 2 {
            match analyze_token_arbitrage_with_states(&parsed_pools) {
                Ok(analysis) => {
                    if analysis.max_profit_ratio > global_max_profit {
                        global_max_profit = analysis.max_profit_ratio;
                        best_token_mint_index = Some(token_group.token_mint_index);
                        best_token_program_index = Some(token_group.token_program_index);
                        best_mint_token_account_index = Some(token_group.mint_token_account_index);
                        best_buy_pool_state = analysis.buy_pool_state.map(Box::new);
                        best_sell_pool_state = analysis.sell_pool_state.map(Box::new);
                    }
                }
                Err(e) => {
                    return Err(e.into());
                }
            }
        }
    }

    // 在直接交易模式下，如果没有找到有效的token组，返回错误
    if is_dir_swap && best_token_mint_index.is_none() {
        return Err(ErrorCode::InvalidTokenPair.into());
    }

    Ok(GlobalArbitrageAnalysis {
        best_token_mint_index,
        max_profit_ratio: global_max_profit,
        buy_pool_state: best_buy_pool_state,
        sell_pool_state: best_sell_pool_state,
        token_program_index: best_token_program_index,
        mint_token_account_index: best_mint_token_account_index,
    })
}

/// 解析池状态并计算价格，返回价格信息和解析好的状态
fn parse_pool_with_state(
    pool_data: &PoolData,
    token_mint_index: usize,
    wsol_mint: Pubkey,
    accounts: &[AccountInfo],
    is_dir_swap: bool,
) -> Result<ParsedPoolState> {
    match pool_data {
        PoolData::CPMM {
            program_id_index,
            pool_index,
            authority_index,
            config_index,
            token0_vault_index,
            token1_vault_index,
            observation_state_index,
        } => {
            let mut pool_state = CpmmPoolState {
                program_id_index: *program_id_index,
                pool_index: *pool_index,
                authority_index: *authority_index,
                observation_state_index: *observation_state_index,
                config_index: *config_index,
                token0_vault_index: *token0_vault_index,
                token1_vault_index: *token1_vault_index,
                token0_reserve: 0,
                token1_reserve: 0,
                token0_mint: *accounts[token_mint_index].key,
                token1_mint: wsol_mint,
                trade_fee_rate: 0,
                price: 0,
                // 初始化费用字段，将在parse_cpmm_pool_data中填充实际值
                protocol_fees_token_0: 0,
                protocol_fees_token_1: 0,
                fund_fees_token_0: 0,
                fund_fees_token_1: 0,
                creator_fee_rate: 0,
                enable_creator_fee: false,
                creator_fee_on: 0,
                creator_fees_token_0: 0,
                creator_fees_token_1: 0,
                total_fee_rate: 0,
            };

            parse_cpmm_pool_data(
                pool_index,
                config_index,
                wsol_mint,
                *accounts[token_mint_index].key,
                accounts,
                &mut pool_state,
            )?;

            // 直接交易模式 不用计算价格 返回当前池状态
            if is_dir_swap {
                return Ok(ParsedPoolState::CPMM { state: pool_state });
            }

            calculate_cpmm_price(wsol_mint, &mut pool_state)?;

            let parsed_state = ParsedPoolState::CPMM { state: pool_state };

            Ok(parsed_state)
        }
        PoolData::DLMM {
            program_id_index,
            event_authority_index,
            oracle_index,
            pool_index,
            reserve_x_index,
            reserve_y_index,
            bin_array_minus_1_index,
            bin_array_0_index,
            bin_array_1_index,
        } => {
            let mut pool_state = DlmmPoolState {
                program_id_index: *program_id_index,
                event_authority_index: *event_authority_index,
                oracle_index: *oracle_index,
                pool_index: *pool_index,
                token_x_mint: *accounts[token_mint_index].key,
                token_y_mint: wsol_mint,
                active_id: 0,
                bin_step: 0,
                base_factor: 0,
                variable_fee_control: 0,
                max_volatility_accumulator: 0,
                base_fee_power_factor: 0,
                total_fee_rate: 0,
                base_fee: 0,
                price: 0,
                volatility_accumulator: 0,
                volatility_reference: 0,
                index_reference: 0,
                // valid_bin_arrays_indexs: vec![],
            };
            parse_dlmm_pool_data(
                pool_index,
                wsol_mint,
                *accounts[token_mint_index].key,
                accounts,
                &mut pool_state,
            )?;

            // 直接交易模式 不用计算价格 返回当前池状态
            if is_dir_swap {
                return Ok(ParsedPoolState::DLMM {
                    state: pool_state,
                    reserve_x_index: *reserve_x_index,
                    reserve_y_index: *reserve_y_index,
                    bin_array_minus_1_index: *bin_array_minus_1_index,
                    bin_array_0_index: *bin_array_0_index,
                    bin_array_1_index: *bin_array_1_index,
                });
            }

            get_dlmm_price_from_bin_array(
                wsol_mint,
                // accounts,
                // *bin_array_0_index,
                &mut pool_state,
            )?;

            calculate_dlmm_total_fee_rate(&mut pool_state)?;

            // 注意：bin array数据延迟解析，只在计算最优交易量时解析
            // 这样可以节省价格比较阶段的CU和内存消耗
            let parsed_state = ParsedPoolState::DLMM {
                state: pool_state,
                reserve_x_index: *reserve_x_index,
                reserve_y_index: *reserve_y_index,
                bin_array_minus_1_index: *bin_array_minus_1_index,
                bin_array_0_index: *bin_array_0_index,
                bin_array_1_index: *bin_array_1_index,
            };

            Ok(parsed_state)
        }
        PoolData::DAMMV2 {
            program_id_index,
            event_authority_index,
            pool_authority_index,
            pool_index,
            token_a_vault_index,
            token_b_vault_index,
        } => {
            let mut pool_state = Dammv2PoolState {
                program_id_index: *program_id_index,
                event_authority_index: *event_authority_index,
                pool_authority_index: *pool_authority_index,
                pool_index: *pool_index,
                token_a_vault_index: *token_a_vault_index,
                token_b_vault_index: *token_b_vault_index,
                token_a_mint: *accounts[token_mint_index].key,
                token_b_mint: wsol_mint,
                sqrt_price: 0,
                cliff_fee_numerator: 0,
                fee_scheduler_mode: 0,
                number_of_period: 0,
                period_frequency: 0,
                reduction_factor: 0,
                liquidity: 0,
                collect_fee_mode: 0,
                dynamic_fee_initialized: 0,
                variable_fee_control: 0,
                bin_step: 0,
                volatility_accumulator: 0,
                activation_type: 0,
                activation_point: 0,
                total_fee_rate: 0,
                price: 0,
            };

            parse_dammv2_pool_data(
                pool_index,
                wsol_mint,
                *accounts[token_mint_index].key,
                accounts,
                &mut pool_state,
            )?;

            // 直接交易模式 不用计算价格 返回当前池状态
            if is_dir_swap {
                return Ok(ParsedPoolState::DAMMV2 { state: pool_state });
            }

            calculate_dammv2_price(&mut pool_state, wsol_mint)?;

            let current_point = match pool_state.activation_type {
                0 => Clock::get()?.slot,
                1 => Clock::get()?.unix_timestamp as u64,
                _ => return Err(ErrorCode::InvalidActivationType.into()),
            };
            calculate_dammv2_total_fee_rate(&mut pool_state, current_point)?;

            let parsed_state = ParsedPoolState::DAMMV2 { state: pool_state };

            Ok(parsed_state)
        }

        PoolData::PUMP {
            program_id_index,
            pool_index,
            global_config_index,
            event_authority_index,
            coin_creator_vault_ata_index,
            coin_creator_vault_authority_index,
            pump_fee_wallet_index,
            pump_fee_wallet_ata_index,
            global_vol_accumulator_index,
            user_vol_accumulator_index,
            system_program_index,
            associated_token_program_index,
            base_vault_index,
            quote_vault_index,
            fee_config_index,
            fee_program_index,
        } => {
            let mut pool_state = PumpPoolState {
                program_id_index: *program_id_index,
                pool_index: *pool_index,
                global_config_index: *global_config_index,
                event_authority_index: *event_authority_index,
                coin_creator_vault_ata_index: *coin_creator_vault_ata_index,
                coin_creator_vault_authority_index: *coin_creator_vault_authority_index,
                pump_fee_wallet_index: *pump_fee_wallet_index,
                pump_fee_wallet_ata_index: *pump_fee_wallet_ata_index,
                global_vol_accumulator_index: *global_vol_accumulator_index,
                user_vol_accumulator_index: *user_vol_accumulator_index,
                system_program_index: *system_program_index,
                associated_token_program_index: *associated_token_program_index,
                base_vault_index: *base_vault_index,
                quote_vault_index: *quote_vault_index,
                base_mint: *accounts[token_mint_index].key,
                quote_mint: wsol_mint,
                trade_fee_rate: 0,
                price: 0,
                base_reserve: 0,
                quote_reserve: 0,
                lp_fee_basis_points: 0,
                protocol_fee_basis_points: 0,
                coin_creator_fee_basis_points: 0,
                fee_config_index: *fee_config_index,
                fee_program_index: *fee_program_index,
                coin_creator: *accounts[token_mint_index].key,
                creator: *accounts[token_mint_index].key,
            };

            parse_pump_pool_data(
                pool_index,
                wsol_mint,
                token_mint_index,
                accounts,
                &mut pool_state,
            )?;

            // 直接交易模式 计算用不到价格 返回当前池状态
            if is_dir_swap {
                return Ok(ParsedPoolState::PUMP { state: pool_state });
            }

            calculate_pump_price(&mut pool_state, wsol_mint)?;

            let parsed_state = ParsedPoolState::PUMP { state: pool_state };

            Ok(parsed_state)
        }
        PoolData::RAYDIUM {
            program_id_index,
            event_authority_index,
            pool_index,
            base_vault_index,
            quote_vault_index,
        } => {
            let mut pool_state = RaydiumPoolState {
                program_id_index: *program_id_index,
                event_authority_index: *event_authority_index,
                pool_index: *pool_index,
                base_vault_index: *base_vault_index,
                quote_vault_index: *quote_vault_index,
                base_mint: *accounts[token_mint_index].key,
                quote_mint: wsol_mint,
                base_reserve: 0,
                quote_reserve: 0,
                trade_fee_rate: 0,
                price: 0,
                swap_fee_numerator: 0,
                swap_fee_denominator: 0,
                base_need_take_pnl: 0,
                quote_need_take_pnl: 0,
            };

            parse_raydium_pool_data(
                pool_index,
                wsol_mint,
                *accounts[token_mint_index].key,
                accounts,
                &mut pool_state,
            )?;

            // 直接交易模式 不用计算价格 返回当前池状态
            if is_dir_swap {
                return Ok(ParsedPoolState::RAYDIUM { state: pool_state });
            }

            calculate_raydium_price(&mut pool_state, wsol_mint)?;

            let parsed_state = ParsedPoolState::RAYDIUM { state: pool_state };

            Ok(parsed_state)
        }
        PoolData::CLMM {
            program_id_index,
            pool_index,
            amm_config_index,
            observation_key_index,
            token_vault_0_index,
            token_vault_1_index,
            tick_array_minus_1_index,
            tick_array_0_index,
            tick_array_1_index,
            bitmap_extension_index,
        } => {
            let mut pool_state = ClmmPoolState {
                program_id_index: *program_id_index,
                pool_index: *pool_index,
                amm_config_index: *amm_config_index,
                observation_key_index: *observation_key_index,
                token_vault_0_index: *token_vault_0_index,
                token_vault_1_index: *token_vault_1_index,
                tick_array_minus_1_index: *tick_array_minus_1_index,
                tick_array_0_index: *tick_array_0_index,
                tick_array_1_index: *tick_array_1_index,
                bitmap_extension_index: *bitmap_extension_index,
                token_mint_0: *accounts[token_mint_index].key,
                token_mint_1: wsol_mint,
                sqrt_price_x64: 0,
                tick_current: 0,
                tick_spacing: 0,
                liquidity: 0,
                trade_fee_rate: 0,
                price: 0,

                tick_array_bitmap: Box::new([0; 16]),
            };


            parse_clmm_pool_data(
                pool_index,
                amm_config_index,
                wsol_mint,
                *accounts[token_mint_index].key,
                accounts,
                &mut pool_state,
            )?;
 
            // 直接交易模式 不用计算价格 返回当前池状态
            if is_dir_swap {
                return Ok(ParsedPoolState::CLMM { state: pool_state });
            }

            calculate_clmm_price(&mut pool_state, wsol_mint)?;

            let parsed_state = ParsedPoolState::CLMM { state: pool_state };

            Ok(parsed_state)
        }
        PoolData::WHIRLPOOL {
            program_id_index,
            pool_index,
            oracle_index,
            vault_a_index,
            vault_b_index,
            tick_array_0_index,
            tick_array_1_index,
            tick_array_2_index,
        } => {
            let mut pool_state = WhirlpoolPoolState {
                program_id_index: *program_id_index,
                pool_index: *pool_index,
                oracle_index: *oracle_index,
                vault_a_index: *vault_a_index,
                vault_b_index: *vault_b_index,
                tick_array_0_index: *tick_array_0_index,
                tick_array_1_index: *tick_array_1_index,
                tick_array_2_index: *tick_array_2_index,
                token_mint_a: *accounts[token_mint_index].key,
                token_mint_b: wsol_mint,
                sqrt_price: 0,
                tick_current_index: 0,
                liquidity: 0,
                trade_fee_rate: 0,
                price: 0,
                tick_spacing: 0,
            };

            parse_whirlpool_pool_data(
                pool_index,
                wsol_mint,
                *accounts[token_mint_index].key,
                accounts,
                &mut pool_state,
            )?;

            let parsed_state = ParsedPoolState::WHIRLPOOL { state: pool_state };

            Ok(parsed_state)
        }
    }
}

#[derive(Debug)]
struct TokenArbitrageAnalysis {
    // pub has_opportunity: bool,
    pub max_profit_ratio: u128,
    pub buy_pool_state: Option<ParsedPoolState>,
    pub sell_pool_state: Option<ParsedPoolState>,
}

fn analyze_token_arbitrage_with_states(
    parsed_pools: &[Box<ParsedPoolState>],
) -> Result<TokenArbitrageAnalysis> {
    if parsed_pools.len() < 2 {
        return Ok(TokenArbitrageAnalysis {
            max_profit_ratio: 0,
            buy_pool_state: None,
            sell_pool_state: None,
        });
    }

    // 🚀 优化1：预处理所有池子的价格和手续费，避免重复转换
    let mut pool_data: Vec<(u128, u128)> = Vec::with_capacity(parsed_pools.len());
    for pool in parsed_pools {
        let (price, fee_q64) = match pool.as_ref() {
            // 注意：手续费统一转成BASE_BPS
            ParsedPoolState::CPMM { state, .. } => (state.price, bps_to_q64(state.total_fee_rate)), //cpmm的total_fee_rate包含creator_fee_rate
            ParsedPoolState::DLMM { state, .. } => {
                (state.price, bps_to_q64(state.total_fee_rate / 1000))
            }
            ParsedPoolState::DAMMV2 { state, .. } => {
                (state.price, bps_to_q64(state.total_fee_rate / 1000))
            }
            ParsedPoolState::PUMP { state, .. } => {
                (state.price, bps_to_q64(state.trade_fee_rate * 100))
            }
            ParsedPoolState::RAYDIUM { state, .. } => {
                (state.price, bps_to_q64(state.trade_fee_rate))
            }
            ParsedPoolState::CLMM { state, .. } => (state.price, bps_to_q64(state.trade_fee_rate)),
            ParsedPoolState::WHIRLPOOL { state, .. } => (state.price, bps_to_q64(state.trade_fee_rate)),
        };
        pool_data.push((price, fee_q64));
    }

    // 🚀 优化2：记录最大利润差值，减少除法计算
    let mut best_buy_idx = 0;
    let mut best_sell_idx = 0;
    let mut best_max_profit_ratio = 0;

    // 双重循环找最优组合
    for i in 0..pool_data.len() {
        let (price_i, fee_i) = pool_data[i];

        for j in 0..pool_data.len() {
            if i == j {
                continue;
            }

            let (price_j, fee_j) = pool_data[j];

            if price_i == 0 || price_j == 0 {
                continue;
            }

            // 快速筛选：价格必须有差异
            if price_j <= price_i {
                continue;
            }

            // 套利条件：P_j/P_i > 1 + fee_i + fee_j + fee_i*fee_j
            // 直接用现有的safe_mul_div_cast函数，内部用U256
            let price_ratio = safe_mul_div_cast(price_j, ONE, price_i, Rounding::Up);
            let threshold = ONE + fee_i + fee_j + q64_mul(fee_i, fee_j); // 对于amm类型精确 但是dlmm不精确
                                                                         // let max_fee = max(fee_i, fee_j);
                                                                         // let threshold = ONE + max_fee; //初略筛选 避免dlmm的精度问题

            if price_ratio > threshold {
                // msg!("price_ratio: {}, threshold_org: {}", price_ratio, threshold_org);
                let profit_diff = price_ratio - threshold;

                if best_max_profit_ratio == 0 {
                    best_buy_idx = i;
                    best_sell_idx = j;
                    best_max_profit_ratio =
                        safe_mul_div_cast(profit_diff, ONE, threshold, Rounding::Up);
                } else {
                    let profit_ratio = safe_mul_div_cast(profit_diff, ONE, threshold, Rounding::Up);
                    if profit_ratio > best_max_profit_ratio {
                        best_buy_idx = i;
                        best_sell_idx = j;
                        best_max_profit_ratio = profit_ratio;
                    }
                }
            }
        }
    }

    // return Ok(TokenArbitrageAnalysis {
    //     max_profit_ratio: 4446744073709551616,
    //     buy_pool_state: Some(parsed_pools[0].as_ref().clone()),
    //     sell_pool_state: Some(parsed_pools[1].as_ref().clone()),
    // });

    // 🚀 优化3：只在找到最优解后才clone，并计算最终利润率
    if best_max_profit_ratio > 0 {
        Ok(TokenArbitrageAnalysis {
            max_profit_ratio: best_max_profit_ratio,
            buy_pool_state: Some(parsed_pools[best_buy_idx].as_ref().clone()),
            sell_pool_state: Some(parsed_pools[best_sell_idx].as_ref().clone()),
        })
    } else {
        Ok(TokenArbitrageAnalysis {
            max_profit_ratio: 0,
            buy_pool_state: None,
            sell_pool_state: None,
        })
    }
}
