use crate::utils::errors::ErrorCode;
use crate::utils::u64x64_math::{SCALE_OFFSET};
// use crate::utils::u128x128_math::{safe_mul_div_cast, Rounding, integer_sqrt_u128};
use anchor_lang::prelude::*;
use crate::utils::utils::{get_transfer_fee};
 
pub mod cpmm_program_id {
    use super::*;
    declare_id!("CPMMoo8L3F4NbTegBCKVNunggL7H1ZpdTHKxQB5qKP1C");
}


/// CPMM池状态数据结构 (简化版，包含关键字段)
#[derive(Debug, Clone)]
pub struct CpmmPoolState {
    pub program_id_index: usize,

    pub pool_index: usize,

    pub authority_index: usize,

    pub observation_state_index: usize,

    pub config_index: usize,
    /// Token0金库
    pub token0_vault_index: usize,
    /// Token1金库
    pub token1_vault_index: usize,

    pub token0_reserve: u64,
    pub token1_reserve: u64,

    /// Token0 mint
    pub token0_mint: Pubkey,
    /// Token1 mint
    pub token1_mint: Pubkey,
    /// 交易费率 (1000000=100%)
    pub trade_fee_rate: u64,
    /// Creator费率 (1000000=100%)
    pub creator_fee_rate: u64,

    pub price: u128,

    pub enable_creator_fee: bool, // 是否启用creator费
    pub creator_fee_on: u8, // 0: both 1: 在输入时收取 2: 在输出时收取

    // 添加费用字段以正确计算可用流动性
    pub protocol_fees_token_0: u64,
    pub protocol_fees_token_1: u64,
    pub fund_fees_token_0: u64,
    pub fund_fees_token_1: u64,
    pub creator_fees_token_0: u64,
    pub creator_fees_token_1: u64,

    //1000000=100%
    pub total_fee_rate: u64
}

/// 解析CPMM池数据 (根据IDL结构优化的解析函数)
pub fn parse_cpmm_pool_data(
    pool_index: &usize,
    config_index: &usize,
    wsol_mint: Pubkey,
    token_mint: Pubkey,
    accounts: &[AccountInfo],
    pool_state: &mut CpmmPoolState,
) -> Result<()> {
    // 验证账户所有者是CPMM程序
    let pool_account = &accounts[*pool_index];
    let amm_config_account = &accounts[*config_index];

    require!(
        *pool_account.owner == cpmm_program_id::ID,
        ErrorCode::InvalidPoolOwner
    );

    let pool_data = pool_account.data.borrow();

    // 根据CPMM PoolState结构解析数据 (基于IDL偏移量)
    // amm_config(32) + pool_creator(32) = 64字节偏移
    // let token0_vault_key = Pubkey::try_from(&pool_data[72..104]).unwrap();
    // let token1_vault_key = Pubkey::try_from(&pool_data[104..136]).unwrap();
    // 跳过lp_mint(32) = 128+32=160
    let token0_mint = Pubkey::try_from(&pool_data[168..200]).unwrap();
    let token1_mint = Pubkey::try_from(&pool_data[200..232]).unwrap();

    // 优雅地验证池子包含SOL和指定token的配对
    let (_, other_mint) = if token0_mint == wsol_mint {
        (token0_mint, token1_mint)
    } else if token1_mint == wsol_mint {
        (token1_mint, token0_mint)
    } else {
        return Err(ErrorCode::InvalidTokenPair.into());
    };

    // 验证另一个token是我们期望的token
    require!(other_mint == token_mint, ErrorCode::TokenMismatch);

    // 验证amm_config地址匹配
    let amm_config_key = Pubkey::try_from(&pool_data[8..40]).unwrap();
    require!(
        amm_config_key == *amm_config_account.key,
        ErrorCode::InvalidAccount
    );

    //保在{} 减少borrow
    // let token0_vault_data = &accounts[pool_state.token0_vault_index].data.borrow();
    // let token1_vault_data = &accounts[pool_state.token1_vault_index].data.borrow();
    pool_state.token0_reserve = {
        let token0_vault_data = &accounts[pool_state.token0_vault_index].data.borrow();
        u64::from_le_bytes(token0_vault_data[64..72].try_into().unwrap())
    };
    pool_state.token1_reserve = {
        let token1_vault_data = &accounts[pool_state.token1_vault_index].data.borrow();
        u64::from_le_bytes(token1_vault_data[64..72].try_into().unwrap())
    };
    
    if pool_state.token0_reserve == 0 || pool_state.token1_reserve == 0 {
        return Err(ErrorCode::ZeroLiquidity.into());
    }

    // 从amm_config读取必要费率
    // let amm_config_data = amm_config_account.data.borrow();
    let (trade_fee_rate, creator_fee_rate) = {  
        let amm_config_data = amm_config_account.data.borrow();
        (u64::from_le_bytes(amm_config_data[12..20].try_into().unwrap()), u64::from_le_bytes(amm_config_data[108..116].try_into().unwrap()))
    };
    // let creator_fee_rate = u64::from_le_bytes(amm_config_data[108..116].try_into().unwrap());  
    
    
    pool_state.creator_fee_rate = creator_fee_rate;
    pool_state.token0_mint = token0_mint;
    pool_state.token1_mint = token1_mint;

    // 解析所有费用累计 (根据真实pool数据结构)
    pool_state.protocol_fees_token_0 = u64::from_le_bytes(pool_data[341..349].try_into().unwrap());
    pool_state.protocol_fees_token_1 = u64::from_le_bytes(pool_data[349..357].try_into().unwrap());
    pool_state.fund_fees_token_0 = u64::from_le_bytes(pool_data[357..365].try_into().unwrap());
    pool_state.fund_fees_token_1 = u64::from_le_bytes(pool_data[365..373].try_into().unwrap());
    
    // creator_fee_on在偏移量389, enable_creator_fee在390
    pool_state.creator_fee_on = pool_data[389];
    pool_state.enable_creator_fee = pool_data[390] != 0;
    
    // creator_fees在偏移量397和405
    pool_state.creator_fees_token_0 = u64::from_le_bytes(pool_data[397..405].try_into().unwrap());
    pool_state.creator_fees_token_1 = u64::from_le_bytes(pool_data[405..413].try_into().unwrap());

    pool_state.trade_fee_rate = trade_fee_rate;

    pool_state.total_fee_rate  = if pool_state.enable_creator_fee{
        trade_fee_rate + creator_fee_rate
    }else{
        trade_fee_rate
    };

    drop(pool_data);


    Ok(())
}

/// 判断是否在输入时收取creator fee
fn is_creator_fee_on_input(
    pool: &CpmmPoolState,
    input_mint: Pubkey,
) -> bool {
    if !pool.enable_creator_fee {
        return false;
    }
    
    let is_input_token0 = input_mint == pool.token0_mint;
    
    match pool.creator_fee_on {
        0 => true, // BothToken - 总是在输入时收取
        1 => is_input_token0, // OnlyToken0 - 只有输入是token0时在输入收取
        2 => !is_input_token0, // OnlyToken1 - 只有输入是token1时在输入收取
        _ => false,
    }
}

/// 计算扣除费用后的实际可用流动性 (模仿Raydium官方实现)
pub fn vault_amount_without_fee(
    pool_state: &CpmmPoolState
) -> (u64, u64) {
    (
        pool_state.token0_reserve
            .checked_sub(pool_state.protocol_fees_token_0 + pool_state.fund_fees_token_0 + pool_state.creator_fees_token_0)
            .unwrap_or(0),
        pool_state.token1_reserve
            .checked_sub(pool_state.protocol_fees_token_1 + pool_state.fund_fees_token_1 + pool_state.creator_fees_token_1)
            .unwrap_or(0),
    )
}

/// 计算CPMM价格 (返回Q64.64格式) - 使用扣除费用后的实际可用流动性
pub fn calculate_cpmm_price(
    wsol_mint: Pubkey,
    pool_state: &mut CpmmPoolState,
) -> Result<()> {
    // // 从金库账户获取原始储备量
    // let token0_vault_data = &accounts[*token0_vault_index].data.borrow();
    // let token1_vault_data = &accounts[*token1_vault_index].data.borrow();

    // // Token账户的amount字段在偏移量64处 (8字节)
    // let token0_reserve_raw = u64::from_le_bytes(token0_vault_data[64..72].try_into().unwrap());
    // let token1_reserve_raw = u64::from_le_bytes(token1_vault_data[64..72].try_into().unwrap());

    // 🔥 关键修复：使用扣除费用后的实际可用流动性
    let (token0_reserve, token1_reserve) =
        vault_amount_without_fee(pool_state);

    // 确定WSOL和Token的储备量
    let (wsol_reserve, token_reserve) = if pool_state.token0_mint == wsol_mint {
        (token0_reserve, token1_reserve)
    } else {
        (token1_reserve, token0_reserve)
    };

    require!(
        wsol_reserve > 0 && token_reserve > 0,
        ErrorCode::ZeroLiquidity
    );

    // 简化价格计算: WSOL储备 / Token储备 * 小数位调整 (保留这个注释 不要删除)
    // let price = (wsol_reserve as f64 / token_reserve as f64) * 10f64.powi(*token_decimals as i32 - SOL_DECIMALS as i32);

    // Q64.64价格计算: (wsol_reserve << 64) / token_reserve
    // 这给出了每个token值多少WSOL的Q64.64表示
    let price_q64 = u128::from(wsol_reserve)
        .checked_shl(SCALE_OFFSET.into())
        .unwrap()
        .checked_div(token_reserve as u128)
        .unwrap();

    pool_state.price = price_q64;

    // 返回完整的Q64.64格式
    Ok(())
}

/// CPMM池精确输入报价计算 (WSOL输入) - 使用扣除费用后的实际可用流动性
/// 🔥 修复版本：与Raydium官方算法完全一致
pub fn cpmm_quote_exact_input_wsol(
    pool: &CpmmPoolState,
    wsol_mint: Pubkey,
    wsol_amount_in: u64,
    mint_token_info: &AccountInfo,
) -> Result<u64> {
    
    // // 读取原始vault余额
    // let token0_vault_data = token0_vault.data.borrow();
    // let token1_vault_data = token1_vault.data.borrow();

    // let token0_reserve_raw = u64::from_le_bytes(token0_vault_data[64..72].try_into().unwrap());
    // let token1_reserve_raw = u64::from_le_bytes(token1_vault_data[64..72].try_into().unwrap());

    // 🔥 关键修复：使用扣除费用后的实际可用流动性
    let (token0_reserve, token1_reserve) =
        vault_amount_without_fee(pool);

    // 确定WSOL和Token的储备量和方向
    let (wsol_reserve, token_reserve) = if pool.token0_mint == wsol_mint {
        (token0_reserve, token1_reserve)
    } else if pool.token1_mint == wsol_mint {
        (token1_reserve, token0_reserve)
    } else {
        return Err(ErrorCode::InvalidTokenPair.into());
    };

    require!(
        wsol_reserve > 0 && token_reserve > 0,
        ErrorCode::ZeroLiquidity
    );

    // 步骤1: 计算费用
    let trade_fee = (u128::from(wsol_amount_in) * u128::from(pool.trade_fee_rate) + 999999) / 1000000;
    
    // 判断creator fee是否在输入时收取
    let is_creator_fee_on_input = is_creator_fee_on_input(pool, wsol_mint);
    
    let mut token_amount_out = if is_creator_fee_on_input {
        // 在输入时收取creator fee
        let creator_fee = (u128::from(wsol_amount_in) * u128::from(pool.creator_fee_rate) + 999999) / 1000000;
        let wsol_amount_after_fee = u128::from(wsol_amount_in)
            .checked_sub(trade_fee)
            .unwrap()
            .checked_sub(creator_fee)
            .unwrap();
        
        // 计算输出
        let numerator = wsol_amount_after_fee.checked_mul(token_reserve as u128).unwrap();
        let denominator = u128::from(wsol_reserve).checked_add(wsol_amount_after_fee).unwrap();
        let token_amount_out = numerator.checked_div(denominator).unwrap();
        
        token_amount_out
    } else {
        // 只扣除trade fee
        let wsol_amount_after_fee = u128::from(wsol_amount_in).checked_sub(trade_fee).unwrap();
        
        // 计算输出
        let numerator = wsol_amount_after_fee.checked_mul(token_reserve as u128).unwrap();
        let denominator = u128::from(wsol_reserve).checked_add(wsol_amount_after_fee).unwrap();
        let token_amount_out = numerator.checked_div(denominator).unwrap();
        
        token_amount_out
    };
    
    // 步骤2: 如果creator fee不在输入收取，则在输出收取
    if !is_creator_fee_on_input && pool.enable_creator_fee {
        let creator_fee = (token_amount_out * u128::from(pool.creator_fee_rate) + 999999) / 1000000;
        token_amount_out = token_amount_out.checked_sub(creator_fee).unwrap();
    }

    //check token 2022 fee
    let transfer_fee = get_transfer_fee(mint_token_info, token_amount_out as u64)?;
    token_amount_out = match transfer_fee {
        0 => token_amount_out,
        _ => token_amount_out.checked_sub(transfer_fee as u128).unwrap(),
    };
   

    Ok(token_amount_out as u64)
}

/// CPMM池精确输入报价计算 (Token输入) - 使用扣除费用后的实际可用流动性
/// 🔥 修复版本：与Raydium官方算法完全一致
pub fn cpmm_quote_exact_input_token(
    pool: &CpmmPoolState,
    wsol_mint: Pubkey,
    token_amount_in: u64,
    mint_token_info: &AccountInfo,
) -> Result<u64> {

    let transfer_fee = get_transfer_fee(mint_token_info, token_amount_in)?;
    let token_amount_in = match transfer_fee {
        0 => token_amount_in,
        _ => token_amount_in.checked_sub(transfer_fee).unwrap(),
    };

    // 🔥 关键修复：使用扣除费用后的实际可用流动性
    let (token0_reserve, token1_reserve) =
        vault_amount_without_fee(pool);

    // 确定Token和WSOL的储备量和方向
    let (token_reserve, wsol_reserve) = if pool.token0_mint == wsol_mint {
        (token1_reserve, token0_reserve) // Token是token1，WSOL是token0
    } else if pool.token1_mint == wsol_mint {
        (token0_reserve, token1_reserve) // Token是token0，WSOL是token1
    } else {
        return Err(ErrorCode::InvalidTokenPair.into());
    };

    require!(
        token_reserve > 0 && wsol_reserve > 0,
        ErrorCode::ZeroLiquidity
    );

    // 步骤1: 计算费用
    let trade_fee = (u128::from(token_amount_in) * u128::from(pool.trade_fee_rate) + 999999) / 1000000;
    
    // 获取Token mint (非WSOL的那个)
    let token_mint = if pool.token0_mint == wsol_mint {
        pool.token1_mint
    } else {
        pool.token0_mint
    };
    
    // 判断creator fee是否在输入时收取
    let is_creator_fee_on_input = is_creator_fee_on_input(pool, token_mint);
    
    let mut wsol_amount_out = if is_creator_fee_on_input {
        // 在输入时收取creator fee
        let creator_fee = (u128::from(token_amount_in) * u128::from(pool.creator_fee_rate) + 999999) / 1000000;
        let token_amount_after_fee = u128::from(token_amount_in)
            .checked_sub(trade_fee)
            .unwrap()
            .checked_sub(creator_fee)
            .unwrap();
        
        // 计算输出
        let numerator = token_amount_after_fee.checked_mul(wsol_reserve as u128).unwrap();
        let denominator = u128::from(token_reserve).checked_add(token_amount_after_fee).unwrap();
        let wsol_amount_out = numerator.checked_div(denominator).unwrap();
        
        wsol_amount_out
    } else {
        // 只扣除trade fee
        let token_amount_after_fee = u128::from(token_amount_in).checked_sub(trade_fee).unwrap();
        
        // 计算输出
        let numerator = token_amount_after_fee.checked_mul(wsol_reserve as u128).unwrap();
        let denominator = u128::from(token_reserve).checked_add(token_amount_after_fee).unwrap();
        let wsol_amount_out = numerator.checked_div(denominator).unwrap();
        
        wsol_amount_out
    };
    
    // 步骤2: 如果creator fee不在输入收取，则在输出收取
    if !is_creator_fee_on_input && pool.enable_creator_fee {
        let creator_fee = (wsol_amount_out * u128::from(pool.creator_fee_rate) + 999999) / 1000000;
        wsol_amount_out = wsol_amount_out.checked_sub(creator_fee).unwrap();
    }



    Ok(wsol_amount_out as u64)
}





// /// 🎯 精确计算CPMM买入时的最大WSOL输入量
// /// 基于恒定乘积公式 x*y=k，给定目标价格精确计算最大输入量
// /// 公式：max_wsol_input = √(x₀ * y₀ * P_target) - x₀
// pub fn calculate_max_wsol_input_linear_approx(
//     pool: &CpmmPoolState,
//     token0_vault: &AccountInfo,
//     token1_vault: &AccountInfo,
//     wsol_mint: Pubkey,
//     target_price_q64: u128, // Q64.64格式
// ) -> Result<u64> {
//     // 读取原始vault余额
//     let token0_vault_data = token0_vault.data.borrow();
//     let token1_vault_data = token1_vault.data.borrow();

//     let token0_reserve_raw = u64::from_le_bytes(token0_vault_data[64..72].try_into().unwrap());
//     let token1_reserve_raw = u64::from_le_bytes(token1_vault_data[64..72].try_into().unwrap());

//     // 使用扣除费用后的实际可用流动性
//     let (token0_reserve, token1_reserve) =
//         vault_amount_without_fee(pool, token0_reserve_raw, token1_reserve_raw);

//     // 确定WSOL和Token的储备量
//     let (wsol_reserve, token_reserve) = if pool.token0_mint == wsol_mint {
//         (token0_reserve, token1_reserve)
//     } else {
//         (token1_reserve, token0_reserve)
//     };

//     require!(wsol_reserve > 0 && token_reserve > 0, ErrorCode::ZeroLiquidity);

//     // 当前价格
//     let current_price_q64 = pool.price;
    
//     // 防除零检查和价格合理性检查
//     if current_price_q64 == 0 || target_price_q64 <= current_price_q64 {
//         return Ok(0); // 目标价格必须高于当前价格才能买入推高价格
//     }

//     // 🎯 精确公式：基于 x*y=k 和目标价格 P_target = x₁/y₁
//     // x₁ = √(k * P_target) = √(x₀ * y₀ * P_target)
//     // max_wsol_input = x₁ - x₀ = √(x₀ * y₀ * P_target) - x₀
    
//     // 计算 k * P_target = x₀ * y₀ * P_target
//     let k_times_target_price = safe_mul_div_cast(
//         wsol_reserve as u128,
//         token_reserve as u128,
//         1, // 先计算k
//         Rounding::Down,
//     );
    
//     let k_times_target_price = safe_mul_div_cast(
//         k_times_target_price,
//         target_price_q64,
//         ONE, // 结果降精度成整数
//         Rounding::Down,
//     );


//     // 计算 √(k * P_target)
//     let target_wsol_reserve = integer_sqrt_u128(k_times_target_price);
//     // msg!("---------------buy-----------------");
//     // msg!("current_price_q64: {}", current_price_q64);
//     // msg!("target_price_q64: {}", target_price_q64);
//     // msg!("k_times_target_price: {}", k_times_target_price);
//     // msg!("wsol_reserve: {}", wsol_reserve);
//     // msg!("target_wsol_reserve: {}", target_wsol_reserve);
//     // msg!("token_reserve: {}", token_reserve);


//     let max_wsol_input = target_wsol_reserve - wsol_reserve as u128;

//     // 考虑交易费用的影响，需要更多输入才能达到目标价格 大致算法 减小CU消耗
//     // let max_input_with_fees = safe_mul_div_cast(
//     //     max_wsol_input,
//     //     BASE_BPS as u128,
//     //     (BASE_BPS - pool.trade_fee_rate) as u128,
//     //     Rounding::Up,
//     // );

//     msg!("max_wsol_input: {}", max_wsol_input);

//     Ok(max_wsol_input as u64)
// }



// /// 🎯 精确计算CPMM卖出时的最大Token输入量
// /// 基于恒定乘积公式 x*y=k，给定目标价格精确计算最大Token输入量
// /// 公式：max_token_input = y₀ - √(x₀ * y₀ / P_target)
// pub fn calculate_max_token_input_linear_approx(
//     pool: &CpmmPoolState,
//     token0_vault: &AccountInfo,
//     token1_vault: &AccountInfo,
//     wsol_mint: Pubkey,
//     target_price_q64: u128, // Q64.64格式
// ) -> Result<u64> {
//     // 读取原始vault余额
//     let token0_vault_data = token0_vault.data.borrow();
//     let token1_vault_data = token1_vault.data.borrow();

//     let token0_reserve_raw = u64::from_le_bytes(token0_vault_data[64..72].try_into().unwrap());
//     let token1_reserve_raw = u64::from_le_bytes(token1_vault_data[64..72].try_into().unwrap());

//     // 使用扣除费用后的实际可用流动性
//     let (token0_reserve, token1_reserve) =
//         vault_amount_without_fee(pool, token0_reserve_raw, token1_reserve_raw);

//     // 确定WSOL和Token的储备量
//     let (wsol_reserve, token_reserve) = if pool.token0_mint == wsol_mint {
//         (token0_reserve, token1_reserve)
//     } else {
//         (token1_reserve, token0_reserve)
//     };

//     require!(wsol_reserve > 0 && token_reserve > 0, ErrorCode::ZeroLiquidity);

//     // 当前价格
//     let current_price_q64 = pool.price;
    
//     // 防除零检查和价格合理性检查
//     if current_price_q64 == 0 || target_price_q64 >= current_price_q64 {
//         return Ok(0); // 目标价格必须低于当前价格才能卖出压低价格
//     }

//     // 🎯 精确公式：基于 x*y=k 和目标价格 P_target = x₁/y₁
//     // y₁ = √(k / P_target) = √(x₀ * y₀ / P_target)  
//     // max_token_input = y₀ - y₁ = y₀ - √(x₀ * y₀ / P_target)
    
//     // 计算 k / P_target = (x₀ * y₀) / P_target
//     let k = safe_mul_div_cast(
//         wsol_reserve as u128,
//         token_reserve as u128,
//         1,
//         Rounding::Down,
//     );
    
//     let k_div_target_price = safe_mul_div_cast(
//         k,
//         ONE,
//         target_price_q64,
//         Rounding::Down,
//     );

//     // 计算 √(k / P_target)
//     let target_token_reserve = integer_sqrt_u128(k_div_target_price);


//     let max_token_input = target_token_reserve - token_reserve as u128;

//     // 考虑交易费用的影响，需要更多输入才能达到目标价格
//     // let max_input_with_fees = safe_mul_div_cast(
//     //     max_token_input,
//     //     BASE_BPS as u128,
//     //     (BASE_BPS - pool.trade_fee_rate) as u128,
//     //     Rounding::Up,
//     // );

//     // 返回等价的WSOL数量用于优化算法比较
//     let max_token_in_equiv_wsol = safe_mul_div_cast(
//         max_token_input,
//         target_price_q64,
//         ONE,
//         Rounding::Down,
//     );

//     msg!("max_token_in_equiv_wsol: {}", max_token_in_equiv_wsol);

//     Ok(max_token_in_equiv_wsol as u64)
// }






