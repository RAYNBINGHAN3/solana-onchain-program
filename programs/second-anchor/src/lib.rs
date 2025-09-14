use anchor_lang::prelude::*;
use anchor_spl::token_interface::{TokenAccount, TokenInterface, Mint};
use anchor_spl::memo::Memo;
use anchor_lang::solana_program::log::sol_log_compute_units;
 
pub mod constant;
pub mod dex;
pub mod utils;
pub mod swap;
pub mod comparison;
pub mod optimalamt;

use utils::errors::ErrorCode;
use constant::{CPMM_ACCOUNT_COUNT, DLMM_ACCOUNT_COUNT, DAMMV2_ACCOUNT_COUNT, PUMP_ACCOUNT_COUNT, RAYDIUM_ACCOUNT_COUNT, CLMM_ACCOUNT_COUNT, WHIRLPOOL_ACCOUNT_COUNT};
use utils::utils::{get_pool_type, PoolData, PoolType, TokenPoolGroup};
use comparison::analyze_global_arbitrage_opportunities;
use optimalamt::find_optimal_wsol_amount_golden_section;
 
 
 

declare_id!("2dDzSGtvn2d46asoSU733SL6hRRNS2waMrC2rDksSs4s");


/// 账户结构定义
#[derive(Accounts)]
pub struct ComparePrices<'info> {
    /// 支付账户
    #[account(mut)]
    pub payer: Signer<'info>,
    
    ///WSOL mint
    pub wsol_mint: InterfaceAccount<'info, Mint>,

    /// Wsol token account帐号
    #[account(
        mut,
        constraint = wsol_token_account.owner == payer.key(),  
        constraint = wsol_token_account.mint == wsol_mint.key()
    )]
    pub wsol_token_account: InterfaceAccount<'info, TokenAccount>,

    // /// 系统程序
    // pub system_program: Program<'info, System>,

    /// Token程序
    pub token_program: Interface<'info, TokenInterface>,

    /// Token2022程序
    pub token_program_2022: Interface<'info, TokenInterface>,

    /// Memo程序
    pub memo_program: Program<'info, Memo>,

    // // / 关联Token程序
    // pub associated_token_program: Program<'info, AssociatedToken>,
}


#[program]
pub mod zooey_go {
    use super::*;

    /// 比较多个token的多个池并进行全局套利分析
    pub fn zooey<'a, 'b, 'c: 'info, 'info>(
        ctx: Context<'a, 'b, 'c, 'info, ComparePrices<'info>>,
        min_profit: u32, // 最小利润
        is_fail: u8, // 0: 不失败，1: 失败
        is_dir_swap: u8, // 0: onchain计算，1: 直接cpi
        amount_in: u64, //only for is_dir_swap = 1
        is_simulate: u8, // 0: 模拟，1: 实际
    ) -> Result<()> {
        if is_dir_swap == 1 {
            require!(amount_in > 0, ErrorCode::ZeroAmountInput);
        }
        // 解析账户结构: token_mint + token_program + (多个token组的池数据)
        let mut current_index = 0;
        let mut token_groups = Vec::with_capacity(3);

        // 解析多个token组
        while current_index + 3 < ctx.remaining_accounts.len() {
            let token_mint_index = current_index;
            let token_program_index = current_index + 1;
            let mint_token_account_index = current_index + 2;

            // // 判断token program是否为Token2022
            // let is_token_2022 = is_token_2022(ctx.remaining_accounts[token_program_index].key());
            // let memo_program_index = if is_token_2022 { Some(current_index + 3) } else { None };
            
            // let mut pools = Vec::with_capacity(6);
            // current_index += 3 + if is_token_2022 { 1 } else { 0 }; // 跳过mint、program、mint_token_account，token2022还要跳过memo
            
            let mut pools = Vec::with_capacity(6);
            current_index += 3;
            // 解析该token的池数据
            while current_index < ctx.remaining_accounts.len() {
                let pool_program_id = &ctx.remaining_accounts[current_index];

                match get_pool_type(pool_program_id.key()) {
                    Ok(pool_type) => match pool_type {
                        Some(PoolType::CPMM) => {
                            if current_index + CPMM_ACCOUNT_COUNT <= ctx.remaining_accounts.len() {
                                pools.push(PoolData::CPMM {
                                    program_id_index: current_index,
                                    authority_index: current_index + 1,
                                    config_index: current_index + 2,
                                    observation_state_index: current_index + 3,
                                    
                                    pool_index: current_index + 4,
                                    token0_vault_index: current_index + 5,
                                    token1_vault_index: current_index + 6,
                                });
                                current_index += CPMM_ACCOUNT_COUNT;
                            } else {
                                break;
                            }
                        }
                        Some(PoolType::DLMM) => {
                            if current_index + DLMM_ACCOUNT_COUNT <= ctx.remaining_accounts.len() {
                                pools.push(PoolData::DLMM {
                                    program_id_index: current_index,
                                    event_authority_index: current_index + 1,
                                    oracle_index: current_index + 2,

                                    pool_index: current_index + 3,
                                    reserve_x_index: current_index + 4,
                                    reserve_y_index: current_index + 5,
                                    bin_array_minus_1_index: current_index + 6,
                                    bin_array_0_index: current_index + 7,
                                    bin_array_1_index: current_index + 8,
                                });
                                current_index += DLMM_ACCOUNT_COUNT;
                            } else {
                                break;
                            }
                        }
                        Some(PoolType::DAMMV2) => {
                            if current_index + DAMMV2_ACCOUNT_COUNT <= ctx.remaining_accounts.len() {
                                pools.push(PoolData::DAMMV2 {
                                    program_id_index: current_index,
                                    event_authority_index: current_index + 1,
                                    pool_authority_index: current_index + 2,
                                    pool_index: current_index + 3,
                                    token_a_vault_index: current_index + 4,
                                    token_b_vault_index: current_index + 5,
                                });
                                current_index += DAMMV2_ACCOUNT_COUNT;
                            } else {
                                break;
                            }
                        }
                        Some(PoolType::PUMP) => {
                            if current_index + PUMP_ACCOUNT_COUNT <= ctx.remaining_accounts.len() {
                                pools.push(PoolData::PUMP {
                                    program_id_index: current_index,
                                    pool_index: current_index + 1,
                                    global_config_index: current_index + 2,
                                    event_authority_index: current_index + 3,
                                    coin_creator_vault_ata_index: current_index + 4,
                                    coin_creator_vault_authority_index: current_index + 5,
                                    pump_fee_wallet_index: current_index + 6,
                                    pump_fee_wallet_ata_index: current_index + 7,
                                    global_vol_accumulator_index: current_index + 8,
                                    user_vol_accumulator_index: current_index + 9,
                                    system_program_index: current_index + 10,
                                    associated_token_program_index: current_index + 11,
                                    base_vault_index: current_index + 12,
                                    quote_vault_index: current_index + 13,
                                    fee_config_index: current_index + 14,
                                    fee_program_index: current_index + 15,
                                });
                                current_index += PUMP_ACCOUNT_COUNT;
                            } else {
                                break;
                            }
                        }
                        Some(PoolType::RAYDIUM) => {
                            if current_index + RAYDIUM_ACCOUNT_COUNT <= ctx.remaining_accounts.len() {
                                pools.push(PoolData::RAYDIUM {
                                    program_id_index: current_index,
                                    event_authority_index: current_index + 1,
                                    pool_index: current_index + 2,
                                    base_vault_index: current_index + 3,
                                    quote_vault_index: current_index + 4,
                                });
                                current_index += RAYDIUM_ACCOUNT_COUNT;
                            } else {
                                break;
                            }
                        },
                        Some(PoolType::CLMM) => {
                            if current_index + CLMM_ACCOUNT_COUNT <= ctx.remaining_accounts.len() {
                                pools.push(PoolData::CLMM {
                                    program_id_index: current_index,
                                    pool_index: current_index + 1,
                                    amm_config_index: current_index + 2,
                                    observation_key_index: current_index + 3,
                                    bitmap_extension_index: current_index + 4,
                                    token_vault_0_index: current_index + 5,
                                    token_vault_1_index: current_index + 6,
                                    tick_array_minus_1_index: current_index + 7,
                                    tick_array_0_index: current_index + 8,
                                    tick_array_1_index: current_index + 9,
                                });
                                current_index += CLMM_ACCOUNT_COUNT;
                            } else {
                                break;
                            }
                        },
                        Some(PoolType::WHIRLPOOL) => {
                            if current_index + WHIRLPOOL_ACCOUNT_COUNT <= ctx.remaining_accounts.len() {
                                pools.push(PoolData::WHIRLPOOL {
                                    program_id_index: current_index,
                                    pool_index: current_index + 1,
                                    oracle_index: current_index + 2,
                                    vault_a_index: current_index + 3,
                                    vault_b_index: current_index + 4,
                                    tick_array_0_index: current_index + 5,
                                    tick_array_1_index: current_index + 6,
                                    tick_array_2_index: current_index + 7,
                                });
                                current_index += WHIRLPOOL_ACCOUNT_COUNT;
                            } else {
                                break;
                            }
                        },
                        None => {
                            // msg!("Unrecognized pool type1: {}", pool_program_id.key());
                            break;
                        }
                    },
                    Err(_) => {
                        break;
                    }
                }
            }

            if !pools.is_empty() {
                // msg!("Mint: {} found {} pools", ctx.remaining_accounts[token_mint_index].key(), pools.len());
                token_groups.push(TokenPoolGroup {
                    token_mint_index,
                    token_program_index,
                    mint_token_account_index,
                    pools,
                });
            }
        }
       
        if token_groups.is_empty() {
            return Err(ErrorCode::NoValidMintFound.into());
        }

        // 直接cpi
        let wsol_mint = ctx.accounts.wsol_mint.key();

        if is_dir_swap == 1 {
            to_dir_swap(amount_in, min_profit, wsol_mint, &ctx, &token_groups)?;
            return Ok(());
        }

       

        match analyze_global_arbitrage_opportunities(
            &token_groups,
            wsol_mint,
            &ctx.remaining_accounts,
            false,
        ) {
            Ok(mut analysis) => {
                drop(token_groups);
                if analysis.max_profit_ratio <= 0 {
                    msg!("No opportunities");
                    if is_fail == 1 {
                        return Err(ErrorCode::NoProfit.into());
                    } else {
                        return Ok(());
                    }
                }

                // 获取当前使用的compute unit
                if is_simulate == 1 {                   
                    sol_log_compute_units();
                }

                // 计算最优买入SOL数量
                match calculate_optimal_wsol_amount(
                    &analysis,
                    wsol_mint,
                    &ctx.accounts.wsol_token_account,
                    &ctx.remaining_accounts,
                    min_profit,
                ) {
                    Ok(optimization_result) => {

                        // 获取当前使用的compute unit 使用2 - 1 算出calculate_optimal_wsol_amount使用的compute unit
                        if is_simulate == 1 {
                            sol_log_compute_units();
                        }

                        let initial_wsol_balance = ctx.accounts.wsol_token_account.amount;
                        match swap::swap::execute_arbitrage_swaps(
                            &mut analysis,
                            &optimization_result,
                            &ctx.remaining_accounts,
                            &ctx,
                            initial_wsol_balance,
                            min_profit,
                            wsol_mint,
                        ) {
                            Ok(swap_result) => {
                                if is_simulate == 1 {
                                    msg!("Dolina1: {:?}", swap_result.wsol_in);
                                    msg!("Dolina2: {:?}", swap_result.profit);
                                    // msg!("Dolina3: {:?}", swap_result.profit);
                                    return Ok(());
                                }
                            }
                            Err(e) => {
                                return Err(e.into());
                            }
                        }
                    }
                    Err(e) => {
                        if is_fail == 1 {
                            return Err(e.into());
                            // return Err(ErrorCode::NoProfit.into());
                        } else {
                            msg!("No opportunities 1");
                            return Ok(());
                        }
                    }
                }
            }
            Err(e) => {
                return Err(e.into());
            }
        }

        Ok(())
    }
}


/// 计算最优买入SOL数量的辅助函数 - 优化版本，减少重复解析
fn calculate_optimal_wsol_amount<'info>(
    analysis: &comparison::GlobalArbitrageAnalysis,
    wsol_mint: Pubkey,
    wsol_token_account: &InterfaceAccount<TokenAccount>,
    accounts: &'info [AccountInfo<'info>],
    min_profit: u32,
) -> Result<optimalamt::OptimizationResult> {
  
    // 直接使用analysis中已解析的池状态数据
    let buy_pool_state = analysis.buy_pool_state.as_ref().unwrap();
    let sell_pool_state = analysis.sell_pool_state.as_ref().unwrap();
    let max_profit_ratio = analysis.max_profit_ratio;
    let token_mint_info = &accounts[analysis.best_token_mint_index.unwrap()];


    find_optimal_wsol_amount_golden_section(
        buy_pool_state,
        sell_pool_state,
        wsol_mint,
        wsol_token_account.amount,
        accounts,
        max_profit_ratio,
        min_profit,
        token_mint_info
    )
}



/// 直接交易模式
fn to_dir_swap<'a, 'b, 'c, 'info>(
    amount_in: u64, 
    min_profit: u32, 
    wsol_mint: Pubkey, 
    ctx: &Context<'a, 'b, 'c, 'info, ComparePrices<'info>>, 
    token_groups: &[TokenPoolGroup]
) -> Result<()> {

    require!(token_groups.len() == 1, ErrorCode::InvalidTokenPair);
    require!(token_groups[0].pools.len() == 2, ErrorCode::InvalidTokenPair);
    
    // 记录初始WSOL余额
    let initial_wsol_balance = ctx.accounts.wsol_token_account.amount;
    
    // 先进行分析，获取池状态数据
    match comparison::analyze_global_arbitrage_opportunities(
        token_groups,
        wsol_mint,
        &ctx.remaining_accounts,
        true,
    ) {
        Ok(mut analysis) => {
            require!(analysis.buy_pool_state.is_some() && analysis.sell_pool_state.is_some(), ErrorCode::NoProfit);
                    // 创建优化结果（直接使用用户指定的金额）
            let mut optimization_result = optimalamt::OptimizationResult {
                optimal_wsol_amount: amount_in,
                max_mint_amount_out: 0, // 这个值会在交换过程中重新计算
                max_profit: 0,
                total_wsol_out: 0,
            };
                  
            // 如果优化后的wsol数量大于余额wsol数量，则使用余额wsol数量
            optimization_result.optimal_wsol_amount = if optimization_result.optimal_wsol_amount > initial_wsol_balance {
                initial_wsol_balance
            } else {
                optimization_result.optimal_wsol_amount
            };

            // 如果买入池是pump，quote是wsol， 则需要计算base amount in
            if let Some(buy_pool) = analysis.buy_pool_state.as_ref() {
                if let comparison::ParsedPoolState::PUMP { state } = buy_pool.as_ref() {
                    if wsol_mint == state.quote_mint {
                        let max_mint_amount_out = dex::pump::pump_quote_exact_input_wsol(
                            state, 
                            wsol_mint, 
                            amount_in, 
                            &ctx.remaining_accounts[analysis.mint_token_account_index.unwrap()]
                        )?;
                        optimization_result.max_mint_amount_out = max_mint_amount_out;
                    }
                }
            }
            
            // 使用现有的套利交换逻辑
            match swap::swap::execute_arbitrage_swaps(
                &mut analysis,
                &optimization_result,
                &ctx.remaining_accounts,
                ctx,
                initial_wsol_balance,
                min_profit,
                wsol_mint,
            ) {
                Ok(_swap_result) => {}
                Err(e) => {
                    return Err(e.into());
                    // return Err(ErrorCode::None.into()); // 隐藏Noprift错误
                }
            }
        }
        Err(e) => {
             return Err(e.into());
            //  return Err(ErrorCode::None.into()); // 隐藏Noprift错误
        }
    }
    Ok(())
}


