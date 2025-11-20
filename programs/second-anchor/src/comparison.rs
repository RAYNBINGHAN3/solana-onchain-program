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
use crate::dex::whirlpool::{parse_whirlpool_pool_data, WhirlpoolPoolState, calculate_whirlpool_price};
use crate::utils::errors::ErrorCode;
use crate::utils::u128x128_math::{bps_to_q64, q64_mul, safe_mul_div_cast, Rounding};
use crate::utils::u64x64_math::ONE;
use crate::utils::utils::{PoolData, PoolType, TokenPoolGroup};
use anchor_lang::prelude::*;
use crate::constant::WSOL_MINT;

/// 解析好的池状态数据，避免重复解析
#[derive(Clone)]
pub enum ParsedPoolState {
    CPMM {
        state: CpmmPoolState,
    },
    DLMM {
        state: DlmmPoolState,
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
        match &self {
            ParsedPoolState::CPMM { .. } => Some(PoolType::CPMM),
            ParsedPoolState::DLMM { .. } => Some(PoolType::DLMM),
            ParsedPoolState::DAMMV2 { .. } => Some(PoolType::DAMMV2),
            ParsedPoolState::PUMP { .. } => Some(PoolType::PUMP),
            ParsedPoolState::RAYDIUM { .. } => Some(PoolType::RAYDIUM),
            ParsedPoolState::CLMM { .. } => Some(PoolType::CLMM),
            ParsedPoolState::WHIRLPOOL { .. } => Some(PoolType::WHIRLPOOL),
        }
    }

    pub fn get_pool_index(&self) -> usize {
        match &self {
            ParsedPoolState::CPMM { state } => state.pool_index,
            ParsedPoolState::DLMM { state, .. } => state.pool_index,
            ParsedPoolState::DAMMV2 { state, .. } => state.pool_index,
            ParsedPoolState::PUMP { state, .. } => state.pool_index,
            ParsedPoolState::RAYDIUM { state, .. } => state.pool_index,
            ParsedPoolState::CLMM { state, .. } => state.pool_index,
            ParsedPoolState::WHIRLPOOL { state, .. } => state.pool_index,
        }
    }

    pub fn get_pool_tokens(&self) -> ([u8; 32], [u8; 32], bool) {
        match &self {
            ParsedPoolState::CPMM { state } => (state.token0_mint, state.token1_mint, state.has_wsol_pool),
            ParsedPoolState::DLMM { state } => (state.token_x_mint, state.token_y_mint, state.has_wsol_pool),
            ParsedPoolState::DAMMV2 { state } => (state.token_a_mint, state.token_b_mint, state.has_wsol_pool),
            ParsedPoolState::PUMP { state } => (state.base_mint, state.quote_mint, state.has_wsol_pool),
            ParsedPoolState::RAYDIUM { state } => (state.base_mint, state.quote_mint, state.has_wsol_pool),
            ParsedPoolState::CLMM { state } => (state.token_mint_0, state.token_mint_1, state.has_wsol_pool),
            ParsedPoolState::WHIRLPOOL { state } => (state.token_mint_a, state.token_mint_b, state.has_wsol_pool),
        }
    }
}

pub struct MidPoolInfo {
    pub is_mid_a2b: bool,
    pub pool_state: Box<ParsedPoolState>,
    pub token1_mint_index: usize,        // 第一个中间token的mint索引
    pub token2_mint_index: usize,        // 第二个中间token的mint索引
    pub token1_program_index: usize,     // 第一个中间token的program索引
    pub token2_program_index: usize,     // 第二个中间token的program索引
    pub token1_mint_token_account_index: usize, // 第一个中间token的账户索引
    pub token2_mint_token_account_index: usize, // 第二个中间token的账户索引
}

/// 2hop套利的token索引信息
pub struct TwoHopTokenInfo {
    pub token_mint_index: usize,
    pub token_program_index: usize,
    pub mint_token_account_index: usize,
}

/// 全局套利分析结果 (包含解析好的数据，避免重复解析)
// #[derive(Debug)]
pub struct GlobalArbitrageAnalysis {
    // pub has_opportunity: bool,
    pub max_profit_ratio: u128, // Q64.64格式利润率
    // 包含解析好的池状态数据 - 使用Box减少栈使用
    pub buy_pool_state: Option<Box<ParsedPoolState>>,
    pub mid_pool_state: Option<Box<MidPoolInfo>>, // 3hop时包含完整的token索引信息
    pub sell_pool_state: Option<Box<ParsedPoolState>>,
    pub two_hop_token_info: Option<TwoHopTokenInfo>, // 2hop时的token索引信息
}

/// 🚀 栈优化：分析状态结构体
struct AnalysisState {
    global_max_profit: u128,
    best_buy_pool_state: Option<Box<ParsedPoolState>>,
    best_sell_pool_state: Option<Box<ParsedPoolState>>,
    best_mid_pool_state: Option<Box<MidPoolInfo>>,
    best_two_hop_token_info: Option<TwoHopTokenInfo>,
}

/// 分析多token的全局套利机会 - 栈优化版本
pub fn analyze_global_arbitrage_opportunities(
    token_groups: &[TokenPoolGroup],
    accounts: &[AccountInfo],
    is_dir_swap: bool,
) -> Result<GlobalArbitrageAnalysis> {
    // 🚀 栈优化：将大型变量移到堆上
    let mut analysis_state = Box::new(AnalysisState {
        global_max_profit: 0u128,
        best_buy_pool_state: None,
        best_sell_pool_state: None,
        best_mid_pool_state: None,
        best_two_hop_token_info: None,
    });

    // 🚀 栈优化：将主循环逻辑移到单独函数
    process_token_groups(token_groups, accounts, is_dir_swap, &mut analysis_state)?;

    // 在直接交易模式下，如果没有找到有效的token组，返回错误
    if is_dir_swap && analysis_state.best_buy_pool_state.is_none() {
        return Err(ErrorCode::InvalidTokenPair.into());
    }

    Ok(GlobalArbitrageAnalysis {
        max_profit_ratio: analysis_state.global_max_profit,
        buy_pool_state: analysis_state.best_buy_pool_state,
        mid_pool_state: analysis_state.best_mid_pool_state,
        sell_pool_state: analysis_state.best_sell_pool_state,
        two_hop_token_info: analysis_state.best_two_hop_token_info,
    })
}

/// 🚀 栈优化：处理token组的单独函数，减少主函数栈使用
fn process_token_groups(
    token_groups: &[TokenPoolGroup],
    accounts: &[AccountInfo],
    is_dir_swap: bool,
    analysis_state: &mut Box<AnalysisState>,
) -> Result<()> {
    // 🚀 关键优化：提前分流2hop和3hop处理
    let is_3hop_group = token_groups.iter().any(|group| group.token2_mint_index.is_some());
    
    if is_3hop_group {
        // 3hop情况：只处理第一个token组（3hop只有一组）
        if let Some(token_group) = token_groups.first() {
            process_3hop_token_group(token_group, accounts, is_dir_swap, analysis_state)?;
        }
    } else {
        // 2hop情况：处理多个token组
        process_2hop_token_groups(token_groups, accounts, is_dir_swap, analysis_state)?;
    }
    
    Ok(())
}

/// 🚀 栈优化：专门处理2hop的多token组
fn process_2hop_token_groups(
    token_groups: &[TokenPoolGroup],
    accounts: &[AccountInfo],
    is_dir_swap: bool,
    analysis_state: &mut Box<AnalysisState>,
) -> Result<()> {
    for token_group in token_groups {
        // 2hop情况下限制池数量为4个
        let mut parsed_pools: Vec<Box<ParsedPoolState>> = Vec::with_capacity(5);

        // 解析池状态
        for pool_data in &token_group.pools {
            if let Ok(parsed_state) = parse_pool_with_state(
                pool_data,
                accounts,
                is_dir_swap,
            ) {
                parsed_pools.push(parsed_state);
            }
        }

        // 直接交易模式处理
        if is_dir_swap && parsed_pools.len() >= 2 {
            analysis_state.best_buy_pool_state = Some(parsed_pools[0].clone());
            analysis_state.best_sell_pool_state = Some(parsed_pools[1].clone());
            analysis_state.best_two_hop_token_info = Some(TwoHopTokenInfo {
                token_mint_index: token_group.token1_mint_index,
                token_program_index: token_group.token1_program_index,
                mint_token_account_index: token_group.token1_mint_token_account_index,
            });
            return Ok(());
        }

        // 2hop套利分析
        if parsed_pools.len() >= 2 {
            if let Ok(token_analysis) = analyze_2hop_arbitrage_only(&parsed_pools) {
                if token_analysis.max_profit_ratio > analysis_state.global_max_profit {
                    update_best_analysis(token_analysis, token_group, accounts, analysis_state)?;
                }
            }
        }
    }
    Ok(())
}

/// 🚀 栈优化：专门处理3hop的单token组
fn process_3hop_token_group(
    token_group: &TokenPoolGroup,
    accounts: &[AccountInfo],
    is_dir_swap: bool,
    analysis_state: &mut Box<AnalysisState>,
) -> Result<()> {
    // 3hop情况下限制池数量为3个（只需要3个池子就能构成3hop）
    let mut parsed_pools: Vec<Box<ParsedPoolState>> = Vec::with_capacity(4);

    // 解析池状态
    for pool_data in &token_group.pools {
        if let Ok(parsed_state) = parse_pool_with_state(
            pool_data,
            accounts,
            is_dir_swap,
        ) {
            parsed_pools.push(parsed_state);
        }
    }
    
    // 直接交易模式处理
    // if is_dir_swap && parsed_pools.len() >= 3 {
    //     analysis_state.best_buy_pool_state = Some(parsed_pools[0].clone());
    //     analysis_state.best_sell_pool_state = Some(parsed_pools[2].clone());
    //     analysis_state.best_two_hop_token_info = None;
    //     analysis_state.best_mid_pool_state = None;
    //     return Ok(());
    // }

    // 3hop套利分析
    if parsed_pools.len() >= 3 {
        if let Ok(token_analysis) = analyze_3hop_arbitrage_only(&parsed_pools) {
            if token_analysis.max_profit_ratio > analysis_state.global_max_profit {
                update_best_analysis(token_analysis, token_group, accounts, analysis_state)?;
            }
        }
    }
    Ok(())
}

/// 🚀 栈优化：更新最佳分析结果
fn update_best_analysis(
    analysis: TokenArbitrageAnalysis,
    token_group: &TokenPoolGroup,
    accounts: &[AccountInfo],
    analysis_state: &mut Box<AnalysisState>,
) -> Result<()> {
    analysis_state.global_max_profit = analysis.max_profit_ratio;
    
    if let Some(mid_state) = &analysis.mid_pool_state {
        // 3hop处理
        let buy_state = analysis.buy_pool_state.as_ref().unwrap();
        let buy_pool_tokens = buy_state.get_pool_tokens();
        
        let token1_mint = if buy_pool_tokens.0 == WSOL_MINT { 
            buy_pool_tokens.1 
        } else { 
            buy_pool_tokens.0 
        };
        
        let token1_account_key = accounts[token_group.token1_mint_index].key();
        let (token1_indices, token2_indices) = if token1_account_key.to_bytes() == token1_mint {
            (
                (token_group.token1_mint_index, token_group.token1_program_index, token_group.token1_mint_token_account_index),
                (token_group.token2_mint_index.unwrap(), token_group.token2_program_index.unwrap(), token_group.token2_mint_token_account_index.unwrap())
            )
        } else {
            (
                (token_group.token2_mint_index.unwrap(), token_group.token2_program_index.unwrap(), token_group.token2_mint_token_account_index.unwrap()),
                (token_group.token1_mint_index, token_group.token1_program_index, token_group.token1_mint_token_account_index)
            )
        };
        
        analysis_state.best_mid_pool_state = Some(Box::new(MidPoolInfo {
            is_mid_a2b: analysis.is_mid_a2b,
            pool_state: Box::new(mid_state.clone()),
            token1_mint_index: token1_indices.0,
            token2_mint_index: token2_indices.0,
            token1_program_index: token1_indices.1,
            token2_program_index: token2_indices.1,
            token1_mint_token_account_index: token1_indices.2,
            token2_mint_token_account_index: token2_indices.2,
        }));
    } else {
        // 2hop处理
        analysis_state.best_two_hop_token_info = Some(TwoHopTokenInfo {
            token_mint_index: token_group.token1_mint_index,
            token_program_index: token_group.token1_program_index,
            mint_token_account_index: token_group.token1_mint_token_account_index,
        });
        analysis_state.best_mid_pool_state = None;
    }
    
    analysis_state.best_buy_pool_state = analysis.buy_pool_state.map(Box::new);
    analysis_state.best_sell_pool_state = analysis.sell_pool_state.map(Box::new);
    Ok(())
}

/// 🚀 栈优化：返回Box避免大型结构体在栈上
fn parse_pool_with_state(
    pool_data: &PoolData,
    accounts: &[AccountInfo],
    is_dir_swap: bool,
) -> Result<Box<ParsedPoolState>> {
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
                token0_mint: [0; 32],
                token1_mint: [0; 32],
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
                has_wsol_pool: false,
            };

            parse_cpmm_pool_data(
                pool_index,
                config_index,
                accounts,
                &mut pool_state,
            )?;

            // 直接交易模式 不用计算价格 返回当前池状态
            if is_dir_swap {
                return Ok(Box::new(ParsedPoolState::CPMM { state: pool_state }));
            }

            calculate_cpmm_price(&mut pool_state)?;

            Ok(Box::new(ParsedPoolState::CPMM { state: pool_state }))
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
                token_x_mint: [0; 32],
                token_y_mint: [0; 32],
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
                reserve_x_index: *reserve_x_index,
                reserve_y_index: *reserve_y_index,
                bin_array_minus_1_index: *bin_array_minus_1_index,
                bin_array_0_index: *bin_array_0_index,
                bin_array_1_index: *bin_array_1_index,
                has_wsol_pool: false,
                // valid_bin_arrays_indexs: vec![],
            };
            parse_dlmm_pool_data(
                pool_index,
                accounts,
                &mut pool_state,
            )?;

            // 直接交易模式 不用计算价格 返回当前池状态
            if is_dir_swap {
                return Ok(Box::new(ParsedPoolState::DLMM {
                    state: pool_state,
                }));
            }

            get_dlmm_price_from_bin_array(
                &mut pool_state,
            )?;

            calculate_dlmm_total_fee_rate(&mut pool_state)?;

            Ok(Box::new(ParsedPoolState::DLMM {
                state: pool_state,
            }))
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
                token_a_mint: [0; 32],
                token_b_mint: [0; 32],
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
                has_wsol_pool: false,
            };

            parse_dammv2_pool_data(
                pool_index,
                accounts,
                &mut pool_state,
            )?;

            // 直接交易模式 不用计算价格 返回当前池状态
            if is_dir_swap {
                return Ok(Box::new(ParsedPoolState::DAMMV2 { state: pool_state }));
            }

            calculate_dammv2_price(&mut pool_state)?;

            let current_point = match pool_state.activation_type {
                0 => Clock::get()?.slot,
                1 => Clock::get()?.unix_timestamp as u64,
                _ => return Err(ErrorCode::InvalidActivationType.into()),
            };
            calculate_dammv2_total_fee_rate(&mut pool_state, current_point)?;

            Ok(Box::new(ParsedPoolState::DAMMV2 { state: pool_state }))
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
                base_mint: [0; 32],
                quote_mint: [0; 32],
                trade_fee_rate: 0,
                price: 0,
                base_reserve: 0,
                quote_reserve: 0,
                lp_fee_basis_points: 0,
                protocol_fee_basis_points: 0,
                coin_creator_fee_basis_points: 0,
                fee_config_index: *fee_config_index,
                fee_program_index: *fee_program_index,
                coin_creator: *accounts[*global_config_index].key,
                creator: *accounts[*global_config_index].key,
                has_wsol_pool: false,
            };

            parse_pump_pool_data(
                pool_index,
                accounts,
                // &token_mint_index,
                &mut pool_state,
            )?;

            // 直接交易模式 计算用不到价格 返回当前池状态
            if is_dir_swap {
                return Ok(Box::new(ParsedPoolState::PUMP { state: pool_state }));
            }

            calculate_pump_price(&mut pool_state)?;

            Ok(Box::new(ParsedPoolState::PUMP { state: pool_state }))
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
                base_mint: [0; 32],
                quote_mint: [0; 32],
                base_reserve: 0,
                quote_reserve: 0,
                trade_fee_rate: 0,
                price: 0,
                swap_fee_numerator: 0,
                swap_fee_denominator: 0,
                base_need_take_pnl: 0,
                quote_need_take_pnl: 0,
                has_wsol_pool: false,
            };

            parse_raydium_pool_data(
                pool_index,
                accounts,
                &mut pool_state,
            )?;

            // 直接交易模式 不用计算价格 返回当前池状态
            if is_dir_swap {
                return Ok(Box::new(ParsedPoolState::RAYDIUM { state: pool_state }));
            }

            calculate_raydium_price(&mut pool_state)?;

            Ok(Box::new(ParsedPoolState::RAYDIUM { state: pool_state }))
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
                token_mint_0: [0; 32],
                token_mint_1: [0; 32],
                sqrt_price_x64: 0,
                tick_current: 0,
                tick_spacing: 0,
                liquidity: 0,
                trade_fee_rate: 0,
                price: 0,
                has_wsol_pool: false,
            };


            parse_clmm_pool_data(
                pool_index,
                amm_config_index,
                accounts,
                &mut pool_state,
            )?;
 
            // 直接交易模式 不用计算价格 返回当前池状态
            if is_dir_swap {
                return Ok(Box::new(ParsedPoolState::CLMM { state: pool_state }));
            }

            calculate_clmm_price(&mut pool_state)?;

            Ok(Box::new(ParsedPoolState::CLMM { state: pool_state }))
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
                token_mint_a: [0; 32],
                token_mint_b: [0; 32],
                sqrt_price: 0,
                tick_current_index: 0,
                liquidity: 0,
                trade_fee_rate: 0,
                price: 0,
                tick_spacing: 0,
                has_wsol_pool: false,
            };

            parse_whirlpool_pool_data(
                pool_index,
                accounts,
                &mut pool_state,
            )?;

            calculate_whirlpool_price(&mut pool_state)?;

            Ok(Box::new(ParsedPoolState::WHIRLPOOL { state: pool_state }))
        }
    }
}


struct TokenArbitrageAnalysis {
    // pub has_opportunity: bool,
    pub max_profit_ratio: u128,
    pub buy_pool_state: Option<ParsedPoolState>,
    pub mid_pool_state: Option<ParsedPoolState>,
    pub sell_pool_state: Option<ParsedPoolState>,
    pub is_mid_a2b: bool,
}


/// 🚀 栈优化：专门处理2hop套利分析
fn analyze_2hop_arbitrage_only(
    parsed_pools: &[Box<ParsedPoolState>],
) -> Result<TokenArbitrageAnalysis> {
    // 预处理池子数据，提取价格和手续费
    let mut pool_data: Vec<(u128, u128)> = Vec::with_capacity(parsed_pools.len());
    for pool in parsed_pools {
        let (price, fee_q64) = extract_price_and_fee(pool.as_ref());
        pool_data.push((price, fee_q64));
    }

    // 只进行2hop分析
    find_best_2hop_arbitrage(&pool_data, parsed_pools)
}

/// 🚀 栈优化：专门处理3hop套利分析
fn analyze_3hop_arbitrage_only(
    parsed_pools: &[Box<ParsedPoolState>],
) -> Result<TokenArbitrageAnalysis> {
    // 预处理池子数据，提取价格和手续费
    let mut pool_data: Vec<(u128, u128)> = Vec::with_capacity(parsed_pools.len());
    for pool in parsed_pools {
        let (price, fee_q64) = extract_price_and_fee(pool.as_ref());
        pool_data.push((price, fee_q64));
    }

    // 只进行3hop分析
    if pool_data.len() >= 3 {
        find_best_3hop_arbitrage(&pool_data, parsed_pools)
    } else {
        Ok(TokenArbitrageAnalysis {
            max_profit_ratio: 0,
            buy_pool_state: None,
            sell_pool_state: None,
            mid_pool_state: None,
            is_mid_a2b: false,
        })
    }
}


#[inline]
fn extract_price_and_fee(pool: &ParsedPoolState) -> (u128, u128) {
    match pool {
        ParsedPoolState::CPMM { state } => {
           
            (state.price, bps_to_q64(state.total_fee_rate))
        }
        ParsedPoolState::DLMM { state } => {
           
            (state.price, bps_to_q64(state.total_fee_rate / 1000))
        }
        ParsedPoolState::DAMMV2 { state } => {
           
            (state.price, bps_to_q64(state.total_fee_rate / 1000))
        }
        ParsedPoolState::PUMP { state } => {
          
            (state.price, bps_to_q64(state.trade_fee_rate * 100))
        }
        ParsedPoolState::RAYDIUM { state } => {
         
            (state.price, bps_to_q64(state.trade_fee_rate))
        }
        ParsedPoolState::CLMM { state } => {
          
            (state.price, bps_to_q64(state.trade_fee_rate))
        }
        ParsedPoolState::WHIRLPOOL { state } => {
            (state.price, bps_to_q64(state.trade_fee_rate))
        }
    }
}

/// 2hop套利：WSOL -> Token -> WSOL
fn find_best_2hop_arbitrage(
    pool_data: &[(u128, u128)], 
    parsed_pools: &[Box<ParsedPoolState>]
) -> Result<TokenArbitrageAnalysis> {
    let mut best_buy_idx = 0;
    let mut best_sell_idx = 0;
    let mut best_profit_ratio = 0u128;

    for i in 0..pool_data.len() {
        let (price_buy, fee_buy) = pool_data[i];
        
        for j in 0..pool_data.len() {
            if i == j { continue; }
            
            let (price_sell, fee_sell) = pool_data[j];
            
            if price_buy == 0 || price_sell == 0 { continue; }

            if price_sell <= price_buy { continue; }

            // 2hop路径：buy时用price_buy（token/wsol），sell时用1/price_sell（wsol/token）
            // 利润率 = (1/price_buy) * (1/price_sell) * (1-fee_buy) * (1-fee_sell)
            // 简化为：profit_factor = 1 / (price_buy * price_sell) * (1-fee_buy) * (1-fee_sell)
            
            if let Some(profit_factor) = calculate_2hop_profit_factor(price_buy, price_sell, fee_buy, fee_sell) {
                if profit_factor > ONE && profit_factor > best_profit_ratio {
                    best_profit_ratio = profit_factor;
                    best_buy_idx = i;
                    best_sell_idx = j;
                }
            }
        }
    }

    if best_profit_ratio > ONE {
        Ok(TokenArbitrageAnalysis {
            max_profit_ratio: best_profit_ratio - ONE, // 减去1得到纯利润率
            buy_pool_state: Some(parsed_pools[best_buy_idx].as_ref().clone()),
            sell_pool_state: Some(parsed_pools[best_sell_idx].as_ref().clone()),
            mid_pool_state: None,
            is_mid_a2b: false,
        })
    } else {
        Ok(TokenArbitrageAnalysis {
            max_profit_ratio: 0,
            buy_pool_state: None,
            sell_pool_state: None,
            mid_pool_state: None,
            is_mid_a2b: false,
        })
    }
}

/// 计算2hop利润因子，避免溢出
#[inline]
fn calculate_2hop_profit_factor(price_buy: u128, price_sell: u128, fee_buy: u128, fee_sell: u128) -> Option<u128> {
    // profit = 1 / (price_buy * price_sell) * (1-fee_buy) * (1-fee_sell)
 
    let fee_factor_buy = ONE.checked_sub(fee_buy)?;
    let fee_factor_sell = ONE.checked_sub(fee_sell)?;

    
    // 计算费用因子的乘积
    let fee_factor = q64_mul(fee_factor_buy, fee_factor_sell);

    let price_product = safe_mul_div_cast(price_sell, ONE, price_buy, Rounding::Up);
    if price_product == 0 { return None; }
 
    Some(safe_mul_div_cast(price_product, fee_factor, ONE, Rounding::Down))
}/// 3hop套利路径结构
struct ThreeHopPath {
    wsol_to_token1_idx: usize,    // WSOL -> Token1 池索引
    token1_to_token2_idx: usize,  // Token1 -> Token2 池索引  
    token2_to_wsol_idx: usize,    // Token2 -> WSOL 池索引
    token1_mint: [u8; 32],        // 中间token1
    token2_mint: [u8; 32],        // 中间token2
}

/// 3hop套利：WSOL -> Token1 -> Token2 -> WSOL  
fn find_best_3hop_arbitrage(
    pool_data: &[(u128, u128)], 
    parsed_pools: &[Box<ParsedPoolState>]
) -> Result<TokenArbitrageAnalysis> {
    // 首先找出所有有效的3hop路径
    let valid_paths = find_valid_3hop_paths(parsed_pools)?;
    
    if valid_paths.is_empty() {
        return Ok(TokenArbitrageAnalysis {
            max_profit_ratio: 0,
            buy_pool_state: None,
            mid_pool_state: None,
            sell_pool_state: None,
            is_mid_a2b: false,
        });
    }

    let mut best_profit_ratio = 0u128;
    let mut best_path: Option<&ThreeHopPath> = None;

    // 对每个有效路径计算利润率
    for path in &valid_paths {
        if let Some(profit_ratio) = calculate_3hop_profit_ratio(path, pool_data, parsed_pools) {
            if profit_ratio > best_profit_ratio {
                best_profit_ratio = profit_ratio;
                best_path = Some(path);
            }
        }
    }

    if let Some(path) = best_path {
        let is_mid_a2b = get_pool_trade_direction(&parsed_pools[path.token1_to_token2_idx], path.token1_mint);

        Ok(TokenArbitrageAnalysis {
            max_profit_ratio: best_profit_ratio,
            buy_pool_state: Some(parsed_pools[path.wsol_to_token1_idx].as_ref().clone()),
            mid_pool_state: Some(parsed_pools[path.token1_to_token2_idx].as_ref().clone()),
            sell_pool_state: Some(parsed_pools[path.token2_to_wsol_idx].as_ref().clone()),
            is_mid_a2b,
        })
    } else {
        Ok(TokenArbitrageAnalysis {
            max_profit_ratio: 0,
            buy_pool_state: None,
            mid_pool_state: None,
            sell_pool_state: None,
            is_mid_a2b: false,
        })
    }
}

/// 找出所有有效的3hop路径：WSOL -> Token1 -> Token2 -> WSOL
fn find_valid_3hop_paths(parsed_pools: &[Box<ParsedPoolState>]) -> Result<Vec<ThreeHopPath>> {
    let mut paths = Vec::with_capacity(1); // 🚀 减少初始容量
    
    // 分类池子：WSOL相关池和非WSOL池
    let mut wsol_pools = Vec::with_capacity(3);   // 🚀 减少容量
    let mut token_pools = Vec::with_capacity(1);  // Token1/Token2 池
    
    for (idx, pool) in parsed_pools.iter().enumerate() {
        let (token_a, token_b, has_wsol) = pool.get_pool_tokens();
        
        if has_wsol {
            wsol_pools.push((idx, token_a, token_b));
        } else {
            token_pools.push((idx, token_a, token_b));
        }
    }
    
    // 构建3hop路径：WSOL->Token1->Token2->WSOL
    for (wsol_pool1_idx, token1_a, token1_b) in &wsol_pools {
        let token1 = if token1_a == &WSOL_MINT { *token1_b } else { *token1_a };
        
        for (token_pool_idx, token2_a, token2_b) in &token_pools {
            // 检查token1是否与中间池连接
            let token2 = if *token2_a == token1 {
                *token2_b
            } else if *token2_b == token1 {
                *token2_a  
            } else {
                continue; // 没有连接
            };
            
            // 找到Token2->WSOL的池
            for (wsol_pool2_idx, token3_a, token3_b) in &wsol_pools {
                if *wsol_pool1_idx == *wsol_pool2_idx { continue; }
                
                let token3 = if token3_a == &WSOL_MINT { *token3_b } else { *token3_a };
                
                if token3 == token2 {
                    // 找到完整路径：WSOL->Token1->Token2->WSOL
                    paths.push(ThreeHopPath {
                        wsol_to_token1_idx: *wsol_pool1_idx,
                        token1_to_token2_idx: *token_pool_idx,
                        token2_to_wsol_idx: *wsol_pool2_idx,
                        token1_mint: token1,
                        token2_mint: token2,
                    });
                }
            }
        }
    }
    
    Ok(paths)
}

 

/// 从池状态中提取token mint信息
fn get_pool_trade_direction(pool: &ParsedPoolState, token_in: [u8; 32]) -> bool {
    match pool {
        ParsedPoolState::CPMM { state } => state.token0_mint == token_in,
        ParsedPoolState::DLMM { state } => state.token_x_mint == token_in,
        ParsedPoolState::DAMMV2 { state } => state.token_a_mint == token_in,
        ParsedPoolState::PUMP { state } => state.base_mint == token_in,
        ParsedPoolState::RAYDIUM { state } => state.base_mint == token_in,
        ParsedPoolState::CLMM { state } => state.token_mint_0 == token_in,
        ParsedPoolState::WHIRLPOOL { state } => state.token_mint_a == token_in,
    }
}

/// 计算3hop路径的利润率，考虑正确的价格方向
fn calculate_3hop_profit_ratio(
    path: &ThreeHopPath,
    pool_data: &[(u128, u128)],
    parsed_pools: &[Box<ParsedPoolState>]
) -> Option<u128> {
    // 获取三个池的价格和费用
    let (price1, fee1) = pool_data[path.wsol_to_token1_idx];
    let (price2, fee2) = pool_data[path.token1_to_token2_idx];
    let (price3, fee3) = pool_data[path.token2_to_wsol_idx];
    
    // 根据路径方向调整价格
    // 第一步：WSOL -> Token1，需要 token1/wsol 价格
    let step1_price = get_directional_price(&parsed_pools[path.wsol_to_token1_idx], price1, true, path.token1_mint)?;
    
    // 第二步：Token1 -> Token2，需要 token2/token1 价格  
    let step2_price = get_directional_price(&parsed_pools[path.token1_to_token2_idx], price2, false, path.token2_mint)?;
    
    // 第三步：Token2 -> WSOL，需要 wsol/token2 价格
    let step3_price = get_directional_price(&parsed_pools[path.token2_to_wsol_idx], price3, true, WSOL_MINT)?;
    
    // 计算总利润率：step1_price * step2_price * step3_price * (1-fee1) * (1-fee2) * (1-fee3)
    let fee_factor1 = ONE.checked_sub(fee1)?;
    let fee_factor2 = ONE.checked_sub(fee2)?;
    let fee_factor3 = ONE.checked_sub(fee3)?;
    
 
    // 先计算前两个价格的乘积
    let price_product_12 = safe_mul_div_cast(step1_price, step2_price, ONE, Rounding::Down);
    if price_product_12 == 0 { return None; }
    
    // 再乘以第三个价格
    let price_product = safe_mul_div_cast(price_product_12, step3_price, ONE, Rounding::Down);
    if price_product == 0 { return None; }
    
    // 计算费用因子乘积
    let fee_product_12 = safe_mul_div_cast(fee_factor1, fee_factor2, ONE, Rounding::Down);
    let fee_product = safe_mul_div_cast(fee_product_12, fee_factor3, ONE, Rounding::Down);
    
    // 最终乘积
    let total_factor = safe_mul_div_cast(price_product, fee_product, ONE, Rounding::Down);
    
    if total_factor > ONE {
        Some(total_factor - ONE)
    } else {
        Some(0)
    }
}

/// 根据交易方向获取正确的价格
fn get_directional_price(
    pool: &ParsedPoolState,
    stored_price: u128,
    is_wsol_involved: bool,
    target_token: [u8; 32]
) -> Option<u128> {
    let (token_a, token_b, _) = pool.get_pool_tokens();
    
    if is_wsol_involved {
        // WSOL相关池，stored_price是wsol/token格式
        if target_token == WSOL_MINT {
            // 需要wsol价格，直接使用stored_price
            Some(stored_price)
        } else {
            // 需要token价格，使用1/stored_price
            if stored_price == 0 { return None; }
            Some(safe_mul_div_cast(ONE, ONE, stored_price, Rounding::Down))
        }
    } else {
        // Token/Token池，stored_price是tokenB/tokenA格式
        if target_token == token_b {
            // 需要tokenB价格，直接使用stored_price
            Some(stored_price)
        } else if target_token == token_a {
            // 需要tokenA价格，使用1/stored_price
            if stored_price == 0 { return None; }
            Some(safe_mul_div_cast(ONE, ONE, stored_price, Rounding::Down))
        } else {
            None
        }
    }
}



