use crate::utils::errors::ErrorCode;
use crate::utils::u64x64_math::{ceil_div, SCALE_OFFSET, ceil_div_u128};
use anchor_lang::prelude::*;
 

// // Pump程序ID
// pub const PUMP_PROGRAM_ID: &str = "pAMMBay6oceH9fJKBRHGP5D4bD4sWpmSwMn52FMfXEA";

pub mod pump_program_id {
    use super::*;
    declare_id!("pAMMBay6oceH9fJKBRHGP5D4bD4sWpmSwMn52FMfXEA");
}

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

    pub base_mint: Pubkey,
    pub quote_mint: Pubkey,

    pub trade_fee_rate: u64,
    pub price: u128,

    pub base_reserve: u64,
    pub quote_reserve: u64,

    //Fee 相关
    pub lp_fee_basis_points: u64,
    pub protocol_fee_basis_points: u64,
    pub coin_creator_fee_basis_points: u64,
}


/// 解析 Pump 池数据
/// 🚀 优化版：使用直接字节偏移解析 Pump 池数据，节省 CU
pub fn parse_pump_pool_data(
    pool_index: &usize,
    _wsol_mint: Pubkey,
    _token_mint: Pubkey,
    accounts: &[AccountInfo],
    pool_state: &mut PumpPoolState,
) -> Result<()> {
    let pool_account = &accounts[*pool_index];
    let global_config_account = &accounts[pool_state.global_config_index];
    let base_vault_account = &accounts[pool_state.base_vault_index];
    let quote_vault_account = &accounts[pool_state.quote_vault_index];

    // 🚀 直接字节偏移解析池数据 (跳过 discriminator 8字节)
    let pool_data = pool_account.data.borrow();
    if pool_data.len() < 256 {  // 确保有足够的数据
        return Err(ErrorCode::InvalidAccount.into());
    }

   
    pool_state.base_mint = Pubkey::try_from(&pool_data[43..75]).unwrap();
    pool_state.quote_mint = Pubkey::try_from(&pool_data[75..107]).unwrap();
    
    // 获取 coin_creator 用于费率计算
    let coin_creator = Pubkey::try_from(&pool_data[211..243]).unwrap();
   
    let base_vault_data = base_vault_account.data.borrow();
    let quote_vault_data = quote_vault_account.data.borrow();
    pool_state.base_reserve = u64::from_le_bytes(base_vault_data[64..72].try_into().unwrap());
    pool_state.quote_reserve = u64::from_le_bytes(quote_vault_data[64..72].try_into().unwrap());
    
    if pool_state.base_reserve == 0 || pool_state.quote_reserve == 0 {
        return Err(ErrorCode::ZeroLiquidity.into());
    }
 
    let config_data = global_config_account.data.borrow();
    pool_state.lp_fee_basis_points = u64::from_le_bytes(config_data[40..48].try_into().unwrap());
    pool_state.protocol_fee_basis_points = u64::from_le_bytes(config_data[48..56].try_into().unwrap());
    
    // 检查是否应用 coin creator 费率
    pool_state.coin_creator_fee_basis_points = if coin_creator == Pubkey::default() {
        0
    } else {
        u64::from_le_bytes(config_data[313..321].try_into().unwrap())
    };

    // msg!("Pump pool_state: {:?}", pool_state);

    Ok(())
}



/// 计算 Pump 池价格
pub fn calculate_pump_price(pool_state: &mut PumpPoolState, wsol_mint: Pubkey) -> Result<()> {
    if pool_state.base_reserve == 0 || pool_state.quote_reserve == 0 {
        pool_state.price = 0;
        return Ok(());
    }

    // 确定WSOL和Token的储备量
    let (wsol_reserve, token_reserve) = if pool_state.quote_mint == wsol_mint {
        (pool_state.quote_reserve, pool_state.base_reserve)
    } else {
        (pool_state.base_reserve, pool_state.quote_reserve)
    };

    let price_q64 = u128::from(wsol_reserve)
        .checked_shl(SCALE_OFFSET.into())
        .unwrap()
        .checked_div(token_reserve as u128)
        .unwrap();

    pool_state.price = price_q64;

    // msg!("Pump price: {} SOL", pool_state.price as f64 / ONE as f64);

    // 计算总手续费率 (basis points)
    pool_state.trade_fee_rate = pool_state.lp_fee_basis_points
        + pool_state.protocol_fee_basis_points
        + pool_state.coin_creator_fee_basis_points;

    Ok(())
}


// 用于最终cpi计算实际需要的wsol数量 草泥马 pump 傻逼设计
pub fn pump_buy_base_input_internal(
    pool_state: &PumpPoolState,
    base_amount: u64,
) -> Result<u64> {
    
    let numerator = (pool_state.quote_reserve as u128) * (base_amount as u128);
    let denominator = (pool_state.base_reserve as u128) - (base_amount as u128);

    
    let quote_amount_in = ceil_div_u128(numerator, denominator) as u64;

    let lp_fee = ceil_div(quote_amount_in * pool_state.lp_fee_basis_points, 10000);
    let protocol_fee = ceil_div(quote_amount_in * pool_state.protocol_fee_basis_points, 10000);
    let coin_creator_fee = ceil_div(quote_amount_in * pool_state.coin_creator_fee_basis_points, 10000);
  

    let total_quote = quote_amount_in
        .checked_add(lp_fee)
        .and_then(|x| x.checked_add(protocol_fee))
        .and_then(|x| x.checked_add(coin_creator_fee))
        .ok_or(ErrorCode::MathOverflow)?;

    Ok(total_quote)
}


/// Buy Quote Input: 用指定数量的 quote token 买入 base
/// 对应 SDK 的 buyQuoteInputInternal
pub fn pump_buy_quote_input_internal(
    pool_state: &PumpPoolState,
    quote_amount: u64,
) -> Result<u64> {
    // 返回能得到的 base 数量
    if pool_state.base_reserve == 0 || pool_state.quote_reserve == 0 {
        return Err(ErrorCode::ZeroLiquidity.into());
    }

    // 使用更高精度计算，避免舍入误差
    let denominator = 10000_u128 + pool_state.trade_fee_rate as u128;
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
pub fn pump_sell_base_input_internal(
    pool_state: &PumpPoolState,
    base_amount: u64,
) -> Result<u64> {
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
    let protocol_fee = ceil_div(quote_amount_out * pool_state.protocol_fee_basis_points, 10000);
    let coin_creator_fee = ceil_div(quote_amount_out * pool_state.coin_creator_fee_basis_points, 10000);

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
    wsol_mint: Pubkey,
    wsol_amount: u64,
) -> Result<u64> {
    if pool_state.quote_mint == wsol_mint {
        // WSOL 是 quote，所以这是 buy quote input
        pump_buy_quote_input_internal(pool_state, wsol_amount)
    } else if pool_state.base_mint == wsol_mint {
        // WSOL 是 base，所以这是 sell base input
        pump_sell_base_input_internal(pool_state, wsol_amount)
    } else {
        Err(ErrorCode::InvalidTokenPair.into())
    }
}

/// 便捷函数：根据 Token 方向自动选择正确的函数
pub fn pump_quote_exact_input_token(
    pool_state: &PumpPoolState,
    wsol_mint: Pubkey,
    token_amount: u64,
) -> Result<u64> {
    if pool_state.base_mint != wsol_mint {
        // Token 是 base，所以这是 sell base input
        pump_sell_base_input_internal(pool_state, token_amount)
    } else {
        // Token 是 quote，所以这是 buy quote input
        pump_buy_quote_input_internal(pool_state, token_amount)
    }
}
