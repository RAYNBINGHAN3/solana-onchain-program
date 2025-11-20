use anchor_lang::prelude::*;
use anchor_spl::token_interface::{TokenAccount, TokenInterface, Mint};
 
 
pub mod constant;
pub mod dex;
pub mod utils;
pub mod swap;
pub mod comparison;
pub mod optimalamt;

use utils::errors::ErrorCode;
use utils::utils::{parse_token_groups};
use comparison::analyze_global_arbitrage_opportunities;
use optimalamt::find_optimal_wsol_amount_golden_section;
 
 
declare_id!("11111111111111111111111111111111");

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
    /// CHECK: This is the memo program
    pub memo_program: AccountInfo<'info>,

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
        let token_groups = parse_token_groups(&ctx.remaining_accounts)?;
       
        if token_groups.is_empty() {
            return Err(ErrorCode::NoValidMintFound.into());
        }

        // 直接cpi
        // let _wsol_mint = ctx.accounts.wsol_mint.key();

        // if is_dir_swap == 1 {
        //     to_dir_swap(amount_in, min_profit, wsol_mint, &ctx, &token_groups)?;
        //     return Ok(());
        // }
      

        match analyze_global_arbitrage_opportunities(
            &token_groups,
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
                // if is_simulate == 1 {                   
                //     sol_log_compute_units();
                // }

                // 计算最优买入SOL数量
                match calculate_optimal_wsol_amount(
                    &analysis,
                    &ctx.accounts.wsol_token_account,
                    &ctx.remaining_accounts,
                    min_profit,
                ) {
                    Ok(optimization_result) => {

                        // 获取当前使用的compute unit 使用2 - 1 算出calculate_optimal_wsol_amount使用的compute unit
                        if is_simulate == 1 {
                            // sol_log_compute_units();
                            // msg!("Dolina1: {}", optimization_result.optimal_wsol_amount);
                            // msg!("Dolina2: {}", optimization_result.max_profit);
                            // msg!("Dolina3: {}", optimization_result.max_mint_amount_out);
                            // msg!("Dolina4: {}", &ctx.remaining_accounts[optimization_result.buy_pool_index].key());
                            // msg!("Dolina5: {}", &ctx.remaining_accounts[optimization_result.sell_pool_index].key());
                            return Ok(());
                        }

                        let initial_wsol_balance = ctx.accounts.wsol_token_account.amount;
                        match swap::swap::execute_arbitrage_swaps(
                            &mut analysis,
                            &optimization_result,
                            &ctx.remaining_accounts,
                            &ctx,
                            initial_wsol_balance,
                            min_profit,
                        ) {
                            Ok(_) => {
                                // if is_simulate == 1 {
                                //     msg!("Dolina1: {:?}", swap_result.wsol_in);
                                //     msg!("Dolina2: {:?}", swap_result.profit);
                                //     // msg!("Dolina3: {:?}", swap_result.profit);
                                //     return Ok(());
                                // }
                                return Ok(());
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
    }
}


//总是wsol->t1->t2->wsol
/// 计算最优买入SOL数量的辅助函数 - 优化版本，减少重复解析
fn calculate_optimal_wsol_amount<'info>(
    analysis: &comparison::GlobalArbitrageAnalysis,
    wsol_token_account: &InterfaceAccount<TokenAccount>,
    accounts: &'info [AccountInfo<'info>],
    min_profit: u32,
) -> Result<optimalamt::OptimizationResult> {
    // 直接使用analysis中已解析的池状态数据和token索引信息
    let buy_pool_state = analysis.buy_pool_state.as_ref().unwrap();
    let sell_pool_state = analysis.sell_pool_state.as_ref().unwrap();
    let mid_pool_info = analysis.mid_pool_state.as_ref().map(|s| s.as_ref());
    let max_profit_ratio = analysis.max_profit_ratio;

    find_optimal_wsol_amount_golden_section(
        buy_pool_state,
        mid_pool_info,
        sell_pool_state,
        wsol_token_account.amount,
        accounts,
        max_profit_ratio,
        min_profit,
        analysis.two_hop_token_info.as_ref(),
    )
}



// /// 直接交易模式
// fn to_dir_swap<'a, 'b, 'c, 'info>(
//     amount_in: u64, 
//     min_profit: u32, 
//     _wsol_mint: Pubkey, 
//     ctx: &Context<'a, 'b, 'c, 'info, ComparePrices<'info>>, 
//     token_groups: &[TokenPoolGroup]
// ) -> Result<()> {

//     require!(token_groups.len() == 1, ErrorCode::InvalidTokenPair);
//     require!(token_groups[0].pools.len() == 2, ErrorCode::InvalidTokenPair);
    
//     // 记录初始WSOL余额
//     let initial_wsol_balance = ctx.accounts.wsol_token_account.amount;
    
//     // 先进行分析，获取池状态数据
//     match comparison::analyze_global_arbitrage_opportunities(
//         token_groups,
//         &ctx.remaining_accounts,
//         true,
//     ) {
//         Ok(mut analysis) => {
//             require!(analysis.buy_pool_state.is_some() && analysis.sell_pool_state.is_some(), ErrorCode::NoProfit);
//                     // 创建优化结果（直接使用用户指定的金额）
//             let mut optimization_result = optimalamt::OptimizationResult {
//                 optimal_wsol_amount: amount_in,
//                 max_mint_amount_out: 0, // 这个值会在交换过程中重新计算
//                 max_profit: 0,
//                 total_wsol_out: 0,
//                 buy_pool_index: 0,
//                 mid_pool_index: None,
//                 sell_pool_index: 0,
//             };
                  
//             // 如果优化后的wsol数量大于余额wsol数量，则使用余额wsol数量
//             optimization_result.optimal_wsol_amount = if optimization_result.optimal_wsol_amount > initial_wsol_balance {
//                 initial_wsol_balance
//             } else {
//                 optimization_result.optimal_wsol_amount
//             };

//             // 如果买入池是pump，quote是wsol， 则需要计算base amount in
//             if let Some(buy_pool) = analysis.buy_pool_state.as_ref() {
//                 if let comparison::ParsedPoolState::PUMP { state } = buy_pool.as_ref() {
//                     if WSOL_MINT == state.quote_mint {
//                         let max_mint_amount_out = dex::pump::pump_quote_exact_input_wsol(
//                             state, 
//                             amount_in, 
//                             &ctx.remaining_accounts[analysis.mint_token_account_index.unwrap()]
//                         )?;
//                         optimization_result.max_mint_amount_out = max_mint_amount_out;
//                     }
//                 }
//             }
            
//             // 使用现有的套利交换逻辑
//             match swap::swap::execute_arbitrage_swaps(
//                 &mut analysis,
//                 &optimization_result,
//                 &ctx.remaining_accounts,
//                 ctx,
//                 initial_wsol_balance,
//                 min_profit,
//             ) {
//                 Ok(_swap_result) => {}
//                 Err(e) => {
//                     return Err(e.into());
//                     // return Err(ErrorCode::None.into()); // 隐藏Noprift错误
//                 }
//             }
//         }
//         Err(e) => {
//              return Err(e.into());
//             //  return Err(ErrorCode::None.into()); // 隐藏Noprift错误
//         }
//     }
//     Ok(())
// }


