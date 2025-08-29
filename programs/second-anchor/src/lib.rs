use anchor_lang::prelude::*;
use anchor_spl::token_interface::{TokenAccount, TokenInterface, Mint};
 
 
pub mod constant;
pub mod dex;
pub mod utils;
pub mod swap;
pub mod comparison;
pub mod optimalamt;

use utils::errors::ErrorCode;
use constant::{CPMM_ACCOUNT_COUNT, DLMM_ACCOUNT_COUNT, DAMMV2_ACCOUNT_COUNT, PUMP_ACCOUNT_COUNT};
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

    // // / 关联Token程序
    // pub associated_token_program: Program<'info, AssociatedToken>,
}


#[program]
pub mod zooey_go {
    use super::*;

    /// 比较多个token的多个池并进行全局套利分析
    pub fn zooey<'a, 'b, 'c, 'info>(
        ctx: Context<'a, 'b, 'c, 'info, ComparePrices<'info>>,
        min_profit: u32, // 最小利润
        is_fail: u8, // 0: 不失败，1: 失败
    ) -> Result<()> {
        // 解析账户结构: token_mint + token_program + (多个token组的池数据)
        let mut current_index = 0;
        let mut token_groups = Vec::with_capacity(3);

        // 解析多个token组
        while current_index + 3 < ctx.remaining_accounts.len() {
            let token_mint_index = current_index;
            let token_program_index = current_index + 1;
            let mint_token_account_index = current_index + 2;

            let mut pools = Vec::with_capacity(8);
            current_index += 3; // 跳过mint和program和mint_token_account

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
                                });
                                current_index += PUMP_ACCOUNT_COUNT;
                            } else {
                                break;
                            }
                        }
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

        // msg!("Total {} token groups", token_groups.len());

        if token_groups.is_empty() {
            msg!("No tokens found");
            return Ok(());
        }

        let wsol_mint = ctx.accounts.wsol_mint.key();
        match analyze_global_arbitrage_opportunities(
            &token_groups,
            wsol_mint,
            &ctx.remaining_accounts,
        ) {
            Ok(analysis) => {
                drop(token_groups);
                if analysis.max_profit_ratio <= 0 {
                    msg!("No opportunities");
                    if is_fail == 1 {
                        return Err(ErrorCode::NoProfit.into());
                    } else {
                        return Ok(());
                    }
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
            
                        match swap::swap::execute_arbitrage_swaps(
                            &analysis,
                            &optimization_result,
                            &ctx.remaining_accounts,
                            &ctx
                        ) {
                            Ok(()) => {}
                            Err(e) => {
                                return Err(e.into());
                            }
                        }
                    }
                    Err(_) => {
                        if is_fail == 1 {
                            return Err(ErrorCode::NoProfit.into());
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
fn calculate_optimal_wsol_amount(
    analysis: &comparison::GlobalArbitrageAnalysis,
    wsol_mint: Pubkey,
    wsol_token_account: &InterfaceAccount<TokenAccount>,
    accounts: &[AccountInfo],
    min_profit: u32,
) -> Result<optimalamt::OptimizationResult> {
  
    // 直接使用analysis中已解析的池状态数据
    let buy_pool_state = analysis.buy_pool_state.as_ref().unwrap();
    let sell_pool_state = analysis.sell_pool_state.as_ref().unwrap();
    let max_profit_ratio = analysis.max_profit_ratio;

    find_optimal_wsol_amount_golden_section(
        buy_pool_state,
        sell_pool_state,
        wsol_mint,
        wsol_token_account.amount,
        accounts,
        max_profit_ratio,
        min_profit,
    )
}



