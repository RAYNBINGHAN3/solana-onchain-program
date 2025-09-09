use crate::utils::errors::ErrorCode;
use crate::utils::u64x64_math::{ceil_div_u128, ONE};
use crate::constant::BASE_BPS;
use anchor_lang::prelude::*;
 

 
pub mod raydium_program_id {
    use super::*;
    declare_id!("675kPX9MHTjS2zt1qfr1NYHuzeLXfQM9H24wFSUt1Mp8");
}


#[derive(Debug, Clone)]
pub struct RaydiumPoolState {
    pub program_id_index: usize,
    pub event_authority_index: usize,
    pub pool_index: usize,
    pub base_vault_index: usize,
    pub quote_vault_index: usize,

    pub base_mint: Pubkey,
    pub quote_mint: Pubkey,

    pub base_reserve: u64,
    pub quote_reserve: u64,

    pub trade_fee_rate: u64,

    pub swap_fee_numerator: u64,
    pub swap_fee_denominator: u64,

    pub base_need_take_pnl: u64,
    pub quote_need_take_pnl: u64,

    pub price: u128,
}



pub fn parse_raydium_pool_data(
    pool_index: &usize,
    wsol_mint: Pubkey,
    token_mint: Pubkey,
    accounts: &[AccountInfo],
    pool_state: &mut RaydiumPoolState,
) -> Result<()> {

    // 验证账户所有者是CPMM程序
    let pool_account = &accounts[*pool_index];

    require!(
        *pool_account.owner == raydium_program_id::ID,
        ErrorCode::InvalidPoolOwner
    );

    let pool_data = pool_account.data.borrow();
    
    let base_mint = Pubkey::try_from(&pool_data[400..432]).unwrap();
    let quote_mint = Pubkey::try_from(&pool_data[432..464]).unwrap();
   
    // 优雅地验证池子包含SOL和指定token的配对
    let (_, other_mint) = if base_mint == wsol_mint {
        (base_mint, quote_mint)
    } else if quote_mint == wsol_mint {
        (quote_mint, base_mint)
    } else {
        return Err(ErrorCode::InvalidTokenPair.into());
    };

    // 验证另一个token是我们期望的token
    require!(other_mint == token_mint, ErrorCode::TokenMismatch);

    let base_reserve = {
        let base_vault_data = &accounts[pool_state.base_vault_index].data.borrow();
        u64::from_le_bytes(base_vault_data[64..72].try_into().unwrap())
    };
    let quote_reserve = {
        let quote_vault_data = &accounts[pool_state.quote_vault_index].data.borrow();
        u64::from_le_bytes(quote_vault_data[64..72].try_into().unwrap())
    };
    
    if base_reserve == 0 || quote_reserve == 0 {
        return Err(ErrorCode::ZeroLiquidity.into());
    }

    let swap_fee_numerator = u64::from_le_bytes(pool_data[176..184].try_into().unwrap());
    let swap_fee_denominator = u64::from_le_bytes(pool_data[184..192].try_into().unwrap());
    let base_need_take_pnl = u64::from_le_bytes(pool_data[192..200].try_into().unwrap());
    let quote_need_take_pnl = u64::from_le_bytes(pool_data[200..208].try_into().unwrap());
    
    drop(pool_data);


    pool_state.base_mint = base_mint;
    pool_state.quote_mint = quote_mint;
    pool_state.base_reserve = base_reserve;
    pool_state.quote_reserve = quote_reserve;
    pool_state.swap_fee_numerator = swap_fee_numerator;
    pool_state.swap_fee_denominator = swap_fee_denominator;
    pool_state.base_need_take_pnl = base_need_take_pnl;
    pool_state.quote_need_take_pnl = quote_need_take_pnl;
 
    Ok(())
}


pub fn calculate_raydium_price(pool_state: &mut RaydiumPoolState, wsol_mint: Pubkey) -> Result<()> {
    // 计算价格：根据mint的顺序确定价格方向
    let price = if pool_state.base_mint == wsol_mint {

        u128::from(pool_state.base_reserve)
        .checked_mul(ONE)
        .unwrap()
        .checked_div(u128::from(pool_state.quote_reserve))
        .unwrap()
    } else {
     
        u128::from(pool_state.quote_reserve)
        .checked_mul(ONE)
        .unwrap()
        .checked_div(u128::from(pool_state.base_reserve))
        .unwrap()
    };

    let trade_fee_rate = (pool_state.swap_fee_numerator * BASE_BPS) / pool_state.swap_fee_denominator;
    
    pool_state.price = price;
    pool_state.trade_fee_rate = trade_fee_rate;

    Ok(())
}

/// Raydium AMM quote计算 - WSOL输入，Token输出
pub fn raydium_quote_exact_input_wsol(
    pool_state: &RaydiumPoolState,
    wsol_mint: Pubkey,
    wsol_amount_in: u64,
) -> Result<u64> {
    if wsol_amount_in == 0 {
        return Ok(0);
    }

    // 计算手续费
    let swap_fee = ceil_div_u128(
        wsol_amount_in as u128 * pool_state.swap_fee_numerator as u128,
        pool_state.swap_fee_denominator as u128,
    ) as u64;
    
    let amount_in_after_fee = wsol_amount_in.checked_sub(swap_fee).unwrap();
    if amount_in_after_fee == 0 {
        return Ok(0);
    }

    // 根据mint确定输入输出的reserve
    let (input_reserve, output_reserve) = if pool_state.base_mint == wsol_mint {
        // WSOL -> Token: base_reserve是WSOL，quote_reserve是Token
        (pool_state.base_reserve, pool_state.quote_reserve)
    } else {
        // WSOL -> Token: quote_reserve是WSOL，base_reserve是Token  
        (pool_state.quote_reserve, pool_state.base_reserve)
    };

    // 需要减去pending的PnL
    let (adjusted_input_reserve, adjusted_output_reserve) = if pool_state.base_mint == wsol_mint {
        // WSOL -> Token: input是base, output是quote
        (
            input_reserve.checked_sub(pool_state.base_need_take_pnl).unwrap(),
            output_reserve.checked_sub(pool_state.quote_need_take_pnl).unwrap()
        )
    } else {
        // WSOL -> Token: input是quote, output是base
        (
            input_reserve.checked_sub(pool_state.quote_need_take_pnl).unwrap(),
            output_reserve.checked_sub(pool_state.base_need_take_pnl).unwrap()
        )
    };

    if adjusted_input_reserve == 0 || adjusted_output_reserve == 0 {
        return Err(ErrorCode::ZeroLiquidity.into());
    }

    // 恒定乘积公式：x * y = k
    // amount_out = (amount_in * output_reserve) / (input_reserve + amount_in)
    let denominator = adjusted_input_reserve as u128 + amount_in_after_fee as u128;
    let amount_out = (amount_in_after_fee as u128 * adjusted_output_reserve as u128)
        / denominator;

    Ok(amount_out as u64)
}


/// Raydium AMM quote计算 - Token输入，WSOL输出
pub fn raydium_quote_exact_input_token(
    pool_state: &RaydiumPoolState,
    wsol_mint: Pubkey,
    token_amount_in: u64,
) -> Result<u64> {
    if token_amount_in == 0 {
        return Ok(0);
    }

    // 计算手续费 - 从输入中扣除
    let swap_fee = ceil_div_u128(
        token_amount_in as u128 * pool_state.swap_fee_numerator as u128,
        pool_state.swap_fee_denominator as u128,
    ) as u64;
    
    let amount_in_after_fee = token_amount_in.checked_sub(swap_fee).unwrap();

    // 根据mint确定输入输出的reserve
    let (input_reserve, output_reserve) = if pool_state.base_mint == wsol_mint {
        // Token -> WSOL: quote_reserve是Token，base_reserve是WSOL
        (pool_state.quote_reserve, pool_state.base_reserve)
    } else {
        // Token -> WSOL: base_reserve是Token，quote_reserve是WSOL
        (pool_state.base_reserve, pool_state.quote_reserve)
    };

    // 需要减去pending的PnL
    let (adjusted_input_reserve, adjusted_output_reserve) = if pool_state.base_mint == wsol_mint {
        // Token -> WSOL: input是quote, output是base
        (
            input_reserve.checked_sub(pool_state.quote_need_take_pnl).unwrap(),
            output_reserve.checked_sub(pool_state.base_need_take_pnl).unwrap()
        )
    } else {
        // Token -> WSOL: input是base, output是quote
        (
            input_reserve.checked_sub(pool_state.base_need_take_pnl).unwrap(),
            output_reserve.checked_sub(pool_state.quote_need_take_pnl).unwrap()
        )
    };

    if adjusted_input_reserve == 0 || adjusted_output_reserve == 0 {
        return Err(ErrorCode::ZeroLiquidity.into());
    }

    // 恒定乘积公式：x * y = k
    // amount_out = (amount_in_after_fee * output_reserve) / (input_reserve + amount_in_after_fee)
    let denominator = adjusted_input_reserve as u128 + amount_in_after_fee as u128;
    let amount_out = (amount_in_after_fee as u128 * adjusted_output_reserve as u128)
        / denominator;

    Ok(amount_out as u64)
}



