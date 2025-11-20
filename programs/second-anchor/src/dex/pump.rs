use crate::utils::errors::ErrorCode;
use crate::utils::u64x64_math::{ceil_div, ceil_div_u128, SCALE_OFFSET};
use crate::utils::utils::get_transfer_fee;
use anchor_lang::prelude::*;
use crate::constant::{WSOL_MINT};

// // Pump程序ID
// pub const PUMP_PROGRAM_ID: &str = "pAMMBay6oceH9fJKBRHGP5D4bD4sWpmSwMn52FMfXEA";

pub mod pump_program_id {
    use super::*;
    declare_id!("pAMMBay6oceH9fJKBRHGP5D4bD4sWpmSwMn52FMfXEA"); // pump amm
}

 //pump fun 不是 pump amm 的程序ID
const PUMP_FUN_PROGRAM_ID:Pubkey = Pubkey::new_from_array( [  
    1,  86, 224, 246, 147, 102,  90, 207,
   68, 219,  21, 104, 191,  23,  91, 170,
   81, 137, 203, 151, 245, 210, 255,  59,
  101,  93,  43, 182, 253, 109,  24, 176
]);

/// Pump池状态数据结构
#[derive(Debug, Clone)]
pub struct PumpPoolState {
    pub program_id_index: usize,

    pub pool_index: usize,
    pub global_config_index: usize,
    pub event_authority_index: usize,
    pub coin_creator_vault_ata_index: usize,
    pub coin_creator_vault_authority_index: usize,
    pub pump_fee_wallet_index: usize,
    pub pump_fee_wallet_ata_index: usize,
    pub global_vol_accumulator_index: usize,
    pub user_vol_accumulator_index: usize,
    pub system_program_index: usize,
    pub associated_token_program_index: usize,
    pub base_vault_index: usize,
    pub quote_vault_index: usize,

    pub base_mint: [u8; 32],
    pub quote_mint: [u8; 32],

    pub trade_fee_rate: u64,
    pub price: u128,

    pub base_reserve: u64,
    pub quote_reserve: u64,

    //Fee 相关 - 支持动态费率
    pub lp_fee_basis_points: u64,
    pub protocol_fee_basis_points: u64,
    pub coin_creator_fee_basis_points: u64,
    pub fee_config_index: usize,
    pub fee_program_index: usize,

    // 用于动态费率计算
    pub creator: Pubkey,        // 池子创建者 (用于isPumpPool判断)
    pub coin_creator: Pubkey,   // 代币创建者 (用于费用计算)


    pub has_wsol_pool: bool,
}

/// 在accounts数组中查找指定mint的账户索引
fn find_mint_account_index(accounts: &[AccountInfo], target_mint: &[u8; 32]) -> Result<usize> {
    let target_pubkey = Pubkey::new_from_array(*target_mint);
    
    for (index, account) in accounts.iter().enumerate() {
        if account.key == &target_pubkey {
            return Ok(index);
        }
    }
    
    Err(ErrorCode::InvalidAccount.into())
}

/// 解析 Pump 池数据
/// 🚀 优化版：使用直接字节偏移解析 Pump 池数据，节省 CU
pub fn parse_pump_pool_data(
    pool_index: &usize,
    accounts: &[AccountInfo],
    // token_mint_index: &usize,
    pool_state: &mut PumpPoolState,
) -> Result<()> {
    let pool_account = &accounts[*pool_index];
    // let global_config_account = &accounts[pool_state.global_config_index];

    // 🚀 直接字节偏移解析池数据 (跳过 discriminator 8字节)
    let pool_data = pool_account.data.borrow();
    if pool_data.len() < 256 {
        // 确保有足够的数据
        return Err(ErrorCode::InvalidAccount.into());
    }

    let base_mint = &pool_data[43..75];
    let quote_mint = &pool_data[75..107];

    pool_state.has_wsol_pool = base_mint == WSOL_MINT || quote_mint == WSOL_MINT;
    pool_state.base_mint = base_mint.try_into().unwrap();
    pool_state.quote_mint = quote_mint.try_into().unwrap();

    let mint_from_pump = if pool_state.has_wsol_pool {
       if pool_state.quote_mint == WSOL_MINT{
         pool_state.base_mint
       } else {
         pool_state.quote_mint
       }
    } else {
        pool_state.base_mint
    };

    // 解析 creator (池子创建者) 和 coin_creator (代币创建者)
    pool_state.creator = Pubkey::try_from(&pool_data[11..43]).unwrap();      // creator字段
    pool_state.coin_creator = Pubkey::try_from(&pool_data[211..243]).unwrap(); // coin_creator字段
   

    pool_state.base_reserve = {
        let base_vault_data = &accounts[pool_state.base_vault_index].data.borrow();
        u64::from_le_bytes(base_vault_data[64..72].try_into().unwrap())
    };
    pool_state.quote_reserve = {
        let quote_vault_data = &accounts[pool_state.quote_vault_index].data.borrow();
        u64::from_le_bytes(quote_vault_data[64..72].try_into().unwrap())
    };

    if pool_state.base_reserve == 0 || pool_state.quote_reserve == 0 {
        return Err(ErrorCode::ZeroLiquidity.into());
    }

    drop(pool_data);
    
    // let config_data = global_config_account.data.borrow();
    // let gloabl_lp_fee_basis_points = u64::from_le_bytes(config_data[40..48].try_into().unwrap());
    // let gloabl_protocol_fee_basis_points = u64::from_le_bytes(config_data[48..56].try_into().unwrap());
    // let gloabl_coin_creator_fee_basis_points = u64::from_le_bytes(config_data[313..321].try_into().unwrap());

    // 检查是否有 fee_config 账户（用于动态费率）
    let fee_config_account = &accounts[pool_state.fee_config_index];
    
    // 在accounts数组中查找mint_from_pump对应的账户索引
    let mint_account_index = find_mint_account_index(accounts, &mint_from_pump)?;
    
    let supply = {
        //从accounts中获取mint
        let mint = accounts[mint_account_index].data.borrow();
        u64::from_le_bytes(mint[36..44].try_into().unwrap())
    };
    // msg!("supply: {} mint: {}", supply, accounts[token_mint_index].key);
    
    // 使用动态费率计算 - 在作用域内借用fee_config_data
    let (lp_fee, protocol_fee, creator_fee) = {
        let fee_config_data = fee_config_account.data.borrow();
        compute_dynamic_fees(pool_state, supply, accounts[mint_account_index].key, &fee_config_data)?
    };

    pool_state.lp_fee_basis_points = lp_fee;
    pool_state.protocol_fee_basis_points = protocol_fee;
    pool_state.coin_creator_fee_basis_points = creator_fee;
 

    Ok(())
}

/// 动态费率计算结构体 - 匹配链上数据格式
#[derive(Clone)]
struct FeeTier {
    market_cap_lamports_threshold: u128,  // 注意：IDL中定义的是u128，不是u64！
    lp_fee_bps: u64,
    protocol_fee_bps: u64,
    creator_fee_bps: u64,
}
 

// struct FeeConfig {
//     bump: u8,                    // 1字节
//     admin: Pubkey,               // 32字节  
//     flat_fees: Fees,             // 24字节 (3个u64)
//     fee_tiers: Vec<FeeTier>,     // 动态数组
// }

// struct FeeTier {
//     market_cap_lamports_threshold: u128,  // 16字节！！！(关键发现)
//     fees: Fees,                           // 24字节
// }
// // 每个FeeTier总计: 16 + 24 = 40字节

// struct Fees {
//     lp_fee_bps: u64,      // 8字节
//     protocol_fee_bps: u64, // 8字节  
//     creator_fee_bps: u64,  // 8字节
// }

/// 计算动态费率
fn compute_dynamic_fees(
    pool_state: &PumpPoolState,
    supply: u64,
    token_mint: &Pubkey,
    fee_config_data: &[u8],
) -> Result<(u64, u64, u64)> {
    // 检查是否是 Pump 池 - 使用 creator 而不是 coin_creator
    let is_pump_pool = is_pump_pool(&token_mint, &pool_state.creator);
    // msg!("is_pump_pool: {}, ", is_pump_pool);
    // msg!("flat_fees: {:?}", parse_flat_fees(fee_config_data)?);
    if !is_pump_pool {
        // 解析flat_fees作为非pump池的费率
        let flat_fees = parse_flat_fees(fee_config_data)?;
        return Ok((flat_fees.0, flat_fees.1, flat_fees.2));
    }
    
    // 计算市值 (Market Cap)
    let market_cap = calculate_market_cap(pool_state, supply)?;
    
        // 使用硬编码的费率档位
    let fee_tiers = get_hardcoded_fee_tiers();
    let fees = calculate_fee_tier(&fee_tiers, market_cap);
    
    Ok((fees.0, fees.1, fees.2))
}


/// 解析fee_config中的flat_fees
fn parse_flat_fees(fee_config_data: &[u8]) -> Result<(u64, u64, u64)> {
    if fee_config_data.len() < 65 {
        return Err(ErrorCode::InvalidAccount.into());
    }
    
    // 跳过discriminator(8) + bump(1) + admin(32) = 41字节，到达flat_fees
    let offset = 41;
    
    // 解析flat_fees结构体 (3个u64)
    let lp_fee_bps = u64::from_le_bytes(
        fee_config_data[offset..offset + 8].try_into()
        .unwrap()
    );
    let protocol_fee_bps = u64::from_le_bytes(
        fee_config_data[offset + 8..offset + 16].try_into()
        .unwrap()
    );
    let creator_fee_bps = u64::from_le_bytes(
        fee_config_data[offset + 16..offset + 24].try_into()
        .unwrap()
    );
    
    Ok((lp_fee_bps, protocol_fee_bps, creator_fee_bps))
}
 


fn is_pump_pool(base_mint: &Pubkey, creator: &Pubkey) -> bool {
    let (pda, _) = Pubkey::find_program_address(
        &[b"pool-authority", base_mint.as_ref()],
        &PUMP_FUN_PROGRAM_ID,
    );
    // msg!("pump_pool_authority_pda: {}, pool_creator: {}, match: {}", pda, creator, pda == *creator);
    pda == *creator
}

/// 计算市值 (按照 SDK 逻辑)
/// market_cap = (base_mint_supply * quote_reserve) / base_reserve
fn calculate_market_cap(pool_state: &PumpPoolState, supply: u64) -> Result<u64> {
    if pool_state.base_reserve == 0 {
        return Ok(0);
    }
 
    // market_cap = (supply * quote_reserve) / base_reserve
    let market_cap = (supply as u128)
        .checked_mul(pool_state.quote_reserve as u128)
        .unwrap_or(0)
        .checked_div(pool_state.base_reserve as u128)
        .unwrap_or(0) as u64;

    Ok(market_cap)
}

 
/// 获取硬编码的费率档位
fn get_hardcoded_fee_tiers() -> Vec<FeeTier> {
    vec![
        FeeTier { market_cap_lamports_threshold: 0u128, lp_fee_bps: 2, protocol_fee_bps: 93, creator_fee_bps: 30 },
        FeeTier { market_cap_lamports_threshold: 420000000000u128, lp_fee_bps: 20, protocol_fee_bps: 5, creator_fee_bps: 95 },
        FeeTier { market_cap_lamports_threshold: 1470000000000u128, lp_fee_bps: 20, protocol_fee_bps: 5, creator_fee_bps: 90 },
        FeeTier { market_cap_lamports_threshold: 2460000000000u128, lp_fee_bps: 20, protocol_fee_bps: 5, creator_fee_bps: 85 },
        FeeTier { market_cap_lamports_threshold: 3440000000000u128, lp_fee_bps: 20, protocol_fee_bps: 5, creator_fee_bps: 80 },
        FeeTier { market_cap_lamports_threshold: 4420000000000u128, lp_fee_bps: 20, protocol_fee_bps: 5, creator_fee_bps: 75 },
        FeeTier { market_cap_lamports_threshold: 9820000000000u128, lp_fee_bps: 20, protocol_fee_bps: 5, creator_fee_bps: 70 },
        FeeTier { market_cap_lamports_threshold: 14740000000000u128, lp_fee_bps: 20, protocol_fee_bps: 5, creator_fee_bps: 65 },
        FeeTier { market_cap_lamports_threshold: 19650000000000u128, lp_fee_bps: 20, protocol_fee_bps: 5, creator_fee_bps: 60 },
        FeeTier { market_cap_lamports_threshold: 24560000000000u128, lp_fee_bps: 20, protocol_fee_bps: 5, creator_fee_bps: 55 },
        FeeTier { market_cap_lamports_threshold: 29470000000000u128, lp_fee_bps: 20, protocol_fee_bps: 5, creator_fee_bps: 50 },
        FeeTier { market_cap_lamports_threshold: 34380000000000u128, lp_fee_bps: 20, protocol_fee_bps: 5, creator_fee_bps: 45 },
        FeeTier { market_cap_lamports_threshold: 39300000000000u128, lp_fee_bps: 20, protocol_fee_bps: 5, creator_fee_bps: 40 },
        FeeTier { market_cap_lamports_threshold: 44210000000000u128, lp_fee_bps: 20, protocol_fee_bps: 5, creator_fee_bps: 35 },
        FeeTier { market_cap_lamports_threshold: 49120000000000u128, lp_fee_bps: 20, protocol_fee_bps: 5, creator_fee_bps: 30 },
        FeeTier { market_cap_lamports_threshold: 54030000000000u128, lp_fee_bps: 20, protocol_fee_bps: 5, creator_fee_bps: 28 },
        FeeTier { market_cap_lamports_threshold: 58940000000000u128, lp_fee_bps: 20, protocol_fee_bps: 5, creator_fee_bps: 25 },
        FeeTier { market_cap_lamports_threshold: 63860000000000u128, lp_fee_bps: 20, protocol_fee_bps: 5, creator_fee_bps: 23 },
        FeeTier { market_cap_lamports_threshold: 68770000000000u128, lp_fee_bps: 20, protocol_fee_bps: 5, creator_fee_bps: 20 },
        FeeTier { market_cap_lamports_threshold: 73681000000000u128, lp_fee_bps: 20, protocol_fee_bps: 5, creator_fee_bps: 18 },
        FeeTier { market_cap_lamports_threshold: 78590000000000u128, lp_fee_bps: 20, protocol_fee_bps: 5, creator_fee_bps: 15 },
        FeeTier { market_cap_lamports_threshold: 83500000000000u128, lp_fee_bps: 20, protocol_fee_bps: 5, creator_fee_bps: 13 },
        FeeTier { market_cap_lamports_threshold: 88400000000000u128, lp_fee_bps: 20, protocol_fee_bps: 5, creator_fee_bps: 10 },
        FeeTier { market_cap_lamports_threshold: 93330000000000u128, lp_fee_bps: 20, protocol_fee_bps: 5, creator_fee_bps: 8 },
        FeeTier { market_cap_lamports_threshold: 98240000000000u128, lp_fee_bps: 20, protocol_fee_bps: 5, creator_fee_bps: 5 },
    ]
}

// 移除了parse_flat_fees函数，使用硬编码数据

// 移除了parse_fee_config_tiers函数，使用硬编码数据


/// 根据市值计算适用的费率档位
fn calculate_fee_tier(fee_tiers: &[FeeTier], market_cap: u64) -> (u64, u64, u64) {
    let market_cap_u128 = market_cap as u128;
    
    // 找到适用的费率档位 - 从高到低遍历，找到第一个满足条件的档位
    let first_tier = fee_tiers[0].clone();
    if market_cap_u128 < first_tier.market_cap_lamports_threshold {
        return (first_tier.lp_fee_bps, first_tier.protocol_fee_bps, first_tier.creator_fee_bps);
    }
    
    for tier in fee_tiers.iter().rev() {
        if market_cap_u128 >= tier.market_cap_lamports_threshold {
            return (tier.lp_fee_bps, tier.protocol_fee_bps, tier.creator_fee_bps);
        }
    }

    (25, 5, 0)
}

/// 计算 Pump 池价格
pub fn calculate_pump_price(pool_state: &mut PumpPoolState) -> Result<()> {
    if pool_state.base_reserve == 0 || pool_state.quote_reserve == 0 {
        pool_state.price = 0;
        return Ok(());
    }

    let price_q64 = if pool_state.has_wsol_pool {
        // 确定WSOL和Token的储备量
        let (wsol_reserve, token_reserve) = if pool_state.quote_mint == WSOL_MINT {
            (pool_state.quote_reserve, pool_state.base_reserve)
        } else {
            (pool_state.base_reserve, pool_state.quote_reserve)
        };

        u128::from(wsol_reserve)
            .checked_shl(SCALE_OFFSET.into())
            .unwrap()
            .checked_div(token_reserve as u128)
            .unwrap()
    }else{
        u128::from(pool_state.quote_reserve)
            .checked_shl(SCALE_OFFSET.into())
            .unwrap()
            .checked_div(pool_state.base_reserve as u128)
            .unwrap()
    };

    pool_state.price = price_q64;
 
    pool_state.trade_fee_rate = pool_state.lp_fee_basis_points + pool_state.protocol_fee_basis_points;

    pool_state.trade_fee_rate +=  get_coin_creator_fee_basis_points(pool_state);

    Ok(())
}


fn get_coin_creator_fee_basis_points(pool_state: &PumpPoolState) -> u64 {
    if pool_state.coin_creator != Pubkey::default() {
        return pool_state.coin_creator_fee_basis_points;
    }
    0
}

// 用于最终cpi计算实际需要的wsol数量 草泥马 pump 傻逼设计
pub fn pump_buy_base_input_internal(pool_state: &PumpPoolState, base_amount: u64) -> Result<u64> {
    let numerator = (pool_state.quote_reserve as u128) * (base_amount as u128);
    let denominator = (pool_state.base_reserve as u128) - (base_amount as u128);

    let quote_amount_in = ceil_div_u128(numerator, denominator) as u64;

    let lp_fee = ceil_div(quote_amount_in * pool_state.lp_fee_basis_points, 10000);
    let protocol_fee = ceil_div(
        quote_amount_in * pool_state.protocol_fee_basis_points,
        10000,
    );

    let coin_creator_fee_basis_points = get_coin_creator_fee_basis_points(pool_state);
    let coin_creator_fee = ceil_div(
        quote_amount_in * coin_creator_fee_basis_points,
        10000,
    );

    let total_quote = quote_amount_in
        .checked_add(lp_fee)
        .and_then(|x| x.checked_add(protocol_fee))
        .and_then(|x| x.checked_add(coin_creator_fee))
        .ok_or(ErrorCode::MathOverflow)?;

    Ok(total_quote)
}

/// Buy Quote Input: 用指定数量的 quote token 买入 base
/// 对应 SDK 的 buyQuoteInputInternal
pub fn pump_buy_quote_input_internal(pool_state: &PumpPoolState, quote_amount: u64) -> Result<u64> {
    // 返回能得到的 base 数量
    if pool_state.base_reserve == 0 || pool_state.quote_reserve == 0 {
        return Err(ErrorCode::ZeroLiquidity.into());
    }

    // 使用更高精度计算，避免舍入误差
    let total_fee_rate = pool_state.lp_fee_basis_points
        + pool_state.protocol_fee_basis_points
        + get_coin_creator_fee_basis_points(pool_state);
    let denominator = 10000_u128 + total_fee_rate as u128;
    let effective_quote = (quote_amount as u128 * 10000_u128) / denominator;

    // base_amount_out = base_reserve * effective_quote / (quote_reserve + effective_quote)
    let numerator = (pool_state.base_reserve as u128) * effective_quote;
    let denominator_effective = (pool_state.quote_reserve as u128) + effective_quote;

    // 向下舍入，确保不会超出实际可获得的数量
    let base_amount_out = (numerator / denominator_effective) as u64;

    Ok(base_amount_out)
}

/// Sell Base Input: 卖出指定数量的 base token
/// 对应 SDK 的 sellBaseInputInternal
pub fn pump_sell_base_input_internal(pool_state: &PumpPoolState, base_amount: u64) -> Result<u64> {
    // 返回能得到的 quote 数量
    if pool_state.base_reserve == 0 || pool_state.quote_reserve == 0 {
        return Err(ErrorCode::ZeroLiquidity.into());
    }

    // 先计算理论输出
    // quote_amount_out = quote_reserve * base_amount / (base_reserve + base_amount)
    let numerator = (pool_state.quote_reserve as u128) * (base_amount as u128);
    let denominator = (pool_state.base_reserve as u128) + (base_amount as u128);
    let quote_amount_out = (numerator / denominator) as u64;

    // 计算各项手续费 (都基于理论输出)
    let lp_fee = ceil_div(quote_amount_out * pool_state.lp_fee_basis_points, 10000);
    let protocol_fee = ceil_div(
        quote_amount_out * pool_state.protocol_fee_basis_points,
        10000,
    );

    let coin_creator_fee_basis_points = get_coin_creator_fee_basis_points(pool_state);
    let coin_creator_fee = ceil_div(
        quote_amount_out * coin_creator_fee_basis_points,
        10000,
    );

    // 最终输出 = 理论输出 - 所有手续费
    let total_fees = lp_fee + protocol_fee + coin_creator_fee;

    // 检查手续费是否超过理论输出
    if total_fees >= quote_amount_out {
        return Ok(0); // 手续费太高，实际输出为0
    }

    let final_quote = quote_amount_out - total_fees;
    Ok(final_quote)
}

// buyBaseInputInternal: 买入指定数量的 base token
// buyQuoteInputInternal: 用指定数量的 quote token 买入
// sellBaseInputInternal: 卖出指定数量的 base token
// sellQuoteInputInternal: 想得到指定数量的 quote token
/// 便捷函数：根据 WSOL 方向自动选择正确的函数
pub fn pump_quote_exact_input_wsol(
    pool_state: &PumpPoolState,
    wsol_amount: u64,
    token_mint_info: &AccountInfo,
) -> Result<u64> {
    if wsol_amount == 0 {
        return Ok(0);
    }

    let mut token_amount_out = if pool_state.quote_mint == WSOL_MINT {
        // WSOL 是 quote，所以这是 buy quote input
        pump_buy_quote_input_internal(pool_state, wsol_amount)?
    } else if pool_state.base_mint == WSOL_MINT {
        // WSOL 是 base，所以这是 sell base input
        pump_sell_base_input_internal(pool_state, wsol_amount)?
    } else {
        return Err(ErrorCode::InvalidTokenPair.into());
    };

    let transfer_fee = get_transfer_fee(token_mint_info, token_amount_out)?;
    token_amount_out = match transfer_fee {
        0 => token_amount_out,
        _ => token_amount_out.checked_sub(transfer_fee).unwrap(),
    };

    Ok(token_amount_out)
}

/// 便捷函数：根据 Token 方向自动选择正确的函数
pub fn pump_quote_exact_input_token(
    pool_state: &PumpPoolState,
    token_amount: u64,
    token_mint_info: &AccountInfo,
) -> Result<u64> {
    if token_amount == 0 {
        return Ok(0);
    }

    //check token 2022 fee
    let transfer_fee = get_transfer_fee(token_mint_info, token_amount)?;
    let token_amount = match transfer_fee {
        0 => token_amount,
        _ => token_amount.checked_sub(transfer_fee).unwrap(),
    };

    if pool_state.base_mint != WSOL_MINT {
        // Token 是 base，所以这是 sell base input
        pump_sell_base_input_internal(pool_state, token_amount)
    } else if pool_state.quote_mint == WSOL_MINT {
        // Token 是 quote，所以这是 buy quote input
        pump_buy_quote_input_internal(pool_state, token_amount)
    } else {
        return Err(ErrorCode::InvalidTokenPair.into());
    }
}

/// 便捷函数：根据 Token 方向自动选择正确的函数
pub fn pump_quote_exact_input_token_output_token(
    pool_state: &PumpPoolState,
    is_base_to_quote: bool,
    token_amount: u64,
    token1_mint_info: &AccountInfo,
    token2_mint_info: &AccountInfo,
) -> Result<u64> {
    if token_amount == 0 {
        return Ok(0);
    }

    //check token 2022 fee
    let transfer_fee = get_transfer_fee(token1_mint_info, token_amount)?;
    let token_amount = match transfer_fee {
        0 => token_amount,
        _ => token_amount.checked_sub(transfer_fee).unwrap(),
    };

    let mut token2_amount_out = if is_base_to_quote {
        // Token 是 base，所以这是 sell base input
        pump_sell_base_input_internal(pool_state, token_amount)?  
    } else {
        // Token 是 quote，所以这是 buy quote input
        pump_buy_quote_input_internal(pool_state, token_amount)?
    };

    //check token 2022 fee
    let transfer_fee = get_transfer_fee(token2_mint_info, token2_amount_out as u64)?;
    token2_amount_out = match transfer_fee {
        0 => token2_amount_out,
        _ => token2_amount_out.checked_sub(transfer_fee).unwrap(),
    };

    Ok(token2_amount_out)
    
}
