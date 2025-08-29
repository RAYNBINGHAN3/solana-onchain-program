use anchor_lang::prelude::*;
use crate::utils::errors::ErrorCode;
use crate::utils::u64x64_math::{ONE, get_fee_in_period};
use crate::utils::u128x128_math::{safe_mul_div_cast, Rounding, mul_div_u256};
use ruint::aliases::U256;

// pub const DAMM_V2_PROGRAM_ID: &str = "cpamdpZCGKUy5JxQXB4dcpGPiikHawvSWAd6mEn1sGG";

pub mod dammv2_program_id {
    use super::*;
    declare_id!("cpamdpZCGKUy5JxQXB4dcpGPiikHawvSWAd6mEn1sGG");
}


/// DAMM-V2池状态数据结构 (包含费率计算所需的所有字段)
#[derive(Debug, Clone)]
pub struct Dammv2PoolState {
    pub program_id_index: usize,
    pub event_authority_index: usize,
    pub pool_authority_index: usize,

    pub pool_index: usize,
    pub token_a_vault_index: usize,
    pub token_b_vault_index: usize,

    pub token_a_mint: Pubkey,
    pub token_b_mint: Pubkey,

    // pub sqrt_min_price: u128,
    // pub sqrt_max_price: u128,
    pub sqrt_price: u128,
    
    // 基础费率结构字段
    pub cliff_fee_numerator: u64,
    pub fee_scheduler_mode: u8,
    pub number_of_period: u16,
    pub period_frequency: u64,
    pub reduction_factor: u64,
    
    // Pool核心字段
    pub liquidity: u128,
    pub collect_fee_mode: u8,
    
    // 协议费率字段
    // pub protocol_fee_percent: u8,
    // pub partner_fee_percent: u8,
    // pub referral_fee_percent: u8,

    // 动态费率结构字段
    pub dynamic_fee_initialized: u8,
    // pub max_volatility_accumulator: u32,
    pub variable_fee_control: u32,
    pub bin_step: u16,
    // pub filter_period: u16,
    // pub decay_period: u16,
    // pub dynamic_reduction_factor: u16,
    // pub last_update_timestamp: u64,
    // pub bin_step_u128: u128,
    // pub sqrt_price_reference: u128,
    pub volatility_accumulator: u128,
    // pub volatility_reference: u128,
 

    pub activation_type: u8,
    pub activation_point: u64,

    // 最终总费率 (基点，用于价格比较)
    pub total_fee_rate: u64,

    // 最终价格 (Q64.64格式，用于价格比较)
    pub price: u128,
}



/// 解析DAMM-V2池数据 (完整解析所有费率计算必需字段)
pub fn parse_dammv2_pool_data(
    pool_index: &usize,
    wsol_mint: Pubkey,
    token_mint: Pubkey,
    accounts: &[AccountInfo],
    pool_state: &mut Dammv2PoolState,
) -> Result<()> {
    let pool_account = &accounts[*pool_index];

    require!(
        *pool_account.owner == dammv2_program_id::ID,
        ErrorCode::InvalidPoolOwner
    );

    let pool_data = pool_account.data.borrow();

    // 跳过账户判别符(8字节)
    // let base_offset = 8;

    // 解析PoolFeesStruct (偏移量8开始，总共160字节)
    // BaseFeeStruct (偏移量8开始，40字节)
    let cliff_fee_numerator = u64::from_le_bytes(
        pool_data[8..16].try_into().unwrap()
    );
    let fee_scheduler_mode = pool_data[16];
    // padding_0: [u8; 5] = 9..14
    let number_of_period = u16::from_le_bytes(
        pool_data[22..24].try_into().unwrap()
    );
    let period_frequency = u64::from_le_bytes(
        pool_data[24..32].try_into().unwrap()
    );
    let reduction_factor = u64::from_le_bytes(
        pool_data[32..40].try_into().unwrap()
    );
    // padding_1: u64 = 32..40

    // 协议费率字段 (BaseFeeStruct之后，偏移量48开始)
    // let protocol_fee_percent = pool_data[base_offset + 40];
    // let partner_fee_percent = pool_data[base_offset + 41];
    // let referral_fee_percent = pool_data[base_offset + 42];
    // padding_0: [u8; 5] = 43..48

    // DynamicFeeStruct (偏移量56开始，96字节)
    // let dynamic_offset = 56;
    let dynamic_fee_initialized = pool_data[56];
    // padding: [u8; 7] = 1..8
    // let max_volatility_accumulator = u32::from_le_bytes(
    //     pool_data[dynamic_offset + 8..dynamic_offset + 12].try_into().unwrap()
    // );
    let variable_fee_control = u32::from_le_bytes(
        pool_data[68..72].try_into().unwrap()
    );
    let bin_step = u16::from_le_bytes(
        pool_data[72..74].try_into().unwrap()
    );
    // 其他DynamicFeeStruct字段...
    let volatility_accumulator = u128::from_le_bytes(
        pool_data[120..136].try_into().unwrap()
    );
    // padding_1: [u64; 2] = 96..112

    // 解析Pool主体字段 (偏移量168开始: 8 + 160 = 168)
    let pool_offset = 168;
    
    let token_a_mint = Pubkey::try_from(&pool_data[pool_offset..pool_offset + 32]).unwrap();
    let token_b_mint = Pubkey::try_from(&pool_data[pool_offset + 32..pool_offset + 64]).unwrap();

    // 验证池子包含SOL和指定token的配对
    let (_, other_mint) = if token_a_mint == wsol_mint {
        (token_a_mint, token_b_mint)
    } else if token_b_mint == wsol_mint {
        (token_b_mint, token_a_mint)
    } else {
        return Err(ErrorCode::InvalidTokenPair.into());
    };

    require!(other_mint == token_mint, ErrorCode::TokenMismatch);
    
    // 解析liquidity (偏移量360)
    let liquidity = u128::from_le_bytes(
        pool_data[360..376].try_into().unwrap()
    );
    
    // 🔥 关键修复：如果liquidity为0，直接返回错误，让上层跳过这个池子
    if liquidity == 0 {
        return Err(ErrorCode::ZeroLiquidity.into());
    }
    
    // 解析价格相关字段 (偏移量424开始)
    let sqrt_price = u128::from_le_bytes(
        pool_data[456..472].try_into().unwrap()
    );

    let activation_point = u64::from_le_bytes(
        pool_data[472..480].try_into().unwrap()
    );
    let activation_type = pool_data[480];
    
    // 解析collect_fee_mode (偏移量484)
    let collect_fee_mode = pool_data[484];

 

    // 填充池状态
    pool_state.token_a_mint = token_a_mint;
    pool_state.token_b_mint = token_b_mint;
    // pool_state.sqrt_min_price = sqrt_min_price;
    // pool_state.sqrt_max_price = sqrt_max_price;
    pool_state.sqrt_price = sqrt_price;
    
    // 基础费率字段
    pool_state.cliff_fee_numerator = cliff_fee_numerator;
    pool_state.fee_scheduler_mode = fee_scheduler_mode;
    pool_state.number_of_period = number_of_period;
    pool_state.period_frequency = period_frequency;
    pool_state.reduction_factor = reduction_factor;
 
    
    // 动态费率字段
    pool_state.dynamic_fee_initialized = dynamic_fee_initialized;
  
    pool_state.variable_fee_control = variable_fee_control;
    pool_state.bin_step = bin_step;
 
    pool_state.volatility_accumulator = volatility_accumulator;
  
 
    
    pool_state.activation_type = activation_type;
    pool_state.activation_point = activation_point;
    pool_state.liquidity = liquidity;
    pool_state.collect_fee_mode = collect_fee_mode;

  

    Ok(())
}

/// 计算DAMM-V2价格 (从sqrt_price转换为Q64.64格式)
pub fn calculate_dammv2_price(pool: &mut Dammv2PoolState, wsol_mint: Pubkey) -> Result<()> {
    // sqrt_price是Q64.64格式的平方根价格
    // 要得到实际价格，需要平方：price = sqrt_price^2
    let sqrt_price_q64 = pool.sqrt_price;
    
    // msg!("pool.sqrt_price : {}" , pool.sqrt_price);

    // 在Q64.64格式中，平方操作：(a * a) >> 64
    let mut price_q64 = sqrt_price_q64
        .checked_mul(sqrt_price_q64)
        .unwrap()
        .checked_shr(64)
        .unwrap();

    if price_q64 == 0 {
        return Err(ErrorCode::ZeroPrice.into());
    }   


    // 在Q64.64格式中，倒数 = (2^64)^2 / price = 2^128 / price
    if pool.token_b_mint != wsol_mint {
        price_q64 = (ONE.checked_shl(64).unwrap())
            .checked_div(price_q64)
            .unwrap();
    }

    // 转换为f64用于打印 (price / 2^64)
    // let price_f64 = price_q64 as f64 / ONE as f64;
    // msg!("DAMM-V2价格: {:.8} SOL", price_f64);

    pool.price = price_q64;
    Ok(())
}


/// 计算DAMM-V2总费率 (完全按照官方SDK实现)
pub fn calculate_dammv2_total_fee_rate(
    pool: &mut Dammv2PoolState,
    current_point: u64,
) -> Result<()> {
    // 1. 计算基础费率
    let base_fee_numerator = get_current_base_fee_numerator(
        pool.cliff_fee_numerator,
        pool.fee_scheduler_mode,
        pool.number_of_period,
        pool.period_frequency,
        pool.reduction_factor,
        current_point,
        pool.activation_point,
    )?;

    // 2. 计算动态费率
    let variable_fee = if pool.dynamic_fee_initialized != 0 {
        get_variable_fee(
            pool.volatility_accumulator,
            pool.bin_step,
            pool.variable_fee_control,
        )?
    } else {
        0
    };

    // 3. 计算总费率 (基础费率 + 动态费率)
    let total_fee_numerator = base_fee_numerator
        .checked_add(variable_fee as u64)
        .unwrap();


    // pub const MAX_FEE_NUMERATOR: u64 = 500_000_000; // 50%
    let max_fee_numerator = 500_000_000;
    let final_fee_numerator = std::cmp::min(total_fee_numerator, max_fee_numerator);

    pool.total_fee_rate = final_fee_numerator;

    Ok(())
}

/// 计算当前基础费率分子 (按照官方SDK逻辑)
fn get_current_base_fee_numerator(
    cliff_fee_numerator: u64,
    fee_scheduler_mode: u8,
    number_of_period: u16,
    period_frequency: u64,
    reduction_factor: u64,
    current_point: u64,
    activation_point: u64,
) -> Result<u64> {
    if period_frequency == 0 {
        return Ok(cliff_fee_numerator);
    }

    // 计算周期数
    let period = if current_point < activation_point {
        number_of_period as u64
    } else {
        let elapsed = current_point.checked_sub(activation_point).unwrap();
        let period = elapsed.checked_div(period_frequency).unwrap();
        std::cmp::min(period, number_of_period as u64)
    };

    match fee_scheduler_mode {
        0 => {
            // Linear模式: fee = cliff_fee_numerator - period * reduction_factor
            let fee_numerator = cliff_fee_numerator
                .checked_sub(period.checked_mul(reduction_factor).unwrap())
                .unwrap_or(0);
            Ok(fee_numerator)
        }
        1 => {
            // Exponential模式: fee = cliff_fee_numerator * (1-reduction_factor/10_000)^period
            let period_u16 = u16::try_from(period).unwrap_or(u16::MAX);
            get_fee_in_period(cliff_fee_numerator, reduction_factor, period_u16)
        }
        _ => Err(ErrorCode::InvalidFeeSchedulerMode.into()),
    }
}

/// 计算动态费率 (按照官方SDK逻辑)
fn get_variable_fee(
    volatility_accumulator: u128,
    bin_step: u16,
    variable_fee_control: u32,
) -> Result<u128> {
    // variable_fee = variable_fee_control * (volatility_accumulator * bin_step)^2
    let square_vfa_bin = volatility_accumulator
        .checked_mul(bin_step as u128)
        .unwrap()
        .checked_pow(2)
        .unwrap();

    let v_fee = square_vfa_bin
        .checked_mul(variable_fee_control as u128)
        .unwrap();

    // 向上取整除法: (v_fee + 99_999_999_999) / 100_000_000_000
    let scaled_v_fee = v_fee
        .checked_add(99_999_999_999u128)
        .unwrap()
        .checked_div(100_000_000_000u128)
        .unwrap();

    Ok(scaled_v_fee)
}

// ============== DAMM-V2 Quote计算函数 ==============

/// DAMM-V2 Quote函数 - WSOL输入，Token输出
pub fn dammv2_quote_exact_input_wsol(
    pool: &Dammv2PoolState,
    wsol_mint: Pubkey,
    wsol_amount_in: u64,
    current_point: u64,
) -> Result<u64> {
    if wsol_amount_in == 0 {
        return Ok(0);
    }
    //skip 0 liquidity
    if pool.liquidity == 0 {
        msg!("Skip dammv2 zero liquidity");
        return Ok(0);
    }

    // 确定交易方向：WSOL输入，Token输出
    let a_to_b = if pool.token_a_mint == wsol_mint {
        true // WSOL是TokenA，输出TokenB
    } else if pool.token_b_mint == wsol_mint {
        false // WSOL是TokenB，输出TokenA
    } else {
        return Err(ErrorCode::InvalidTokenPair.into());
    };

    // 获取费用模式
    let fee_mode = get_fee_mode(pool.collect_fee_mode, a_to_b)?;

    // 计算实际输入数量（如果费用在输入上收取）
    let actual_amount_in = if fee_mode.fees_on_input {
        let total_fee = calculate_fee_on_amount_detailed(
            pool,
            wsol_amount_in,
            current_point,
            false, // has_referral
            false, // has_partner
        )?;
        wsol_amount_in.checked_sub(total_fee).unwrap_or(0)
    } else {
        wsol_amount_in
    };

    // 计算交换结果（不含费用）
    let output_amount = if a_to_b {
        get_swap_result_from_a_to_b(pool.sqrt_price, pool.liquidity, actual_amount_in)?
    } else {
        get_swap_result_from_b_to_a(pool.sqrt_price, pool.liquidity, actual_amount_in)?
    };

    // 如果费用在输出上收取，需要扣除费用
    let final_output = if !fee_mode.fees_on_input {
        let total_fee = calculate_fee_on_amount_detailed(
            pool,
            output_amount,
            current_point,
            false, // has_referral
            false, // has_partner
        )?;
        output_amount.checked_sub(total_fee).unwrap_or(0)
    } else {
        output_amount
    };

    Ok(final_output)
}

/// DAMM-V2 Quote函数 - Token输入，WSOL输出 (完全复刻官方SDK)
pub fn dammv2_quote_exact_input_token(
    pool: &Dammv2PoolState,
    wsol_mint: Pubkey,
    token_amount_in: u64,
    current_point: u64,
) -> Result<u64> {
    if token_amount_in == 0 {
        return Ok(0);
    }
    //skip 0 liquidity
    if pool.liquidity == 0 {
        msg!("Skip dammv2 zero liquidity");
        return Ok(0);
    }

    // 确定交易方向：Token输入，WSOL输出
    let a_to_b = if pool.token_a_mint == wsol_mint {
        false // Token是TokenB，输出TokenA(WSOL)
    } else if pool.token_b_mint == wsol_mint {
        true // Token是TokenA，输出TokenB(WSOL)
    } else {
        return Err(ErrorCode::InvalidTokenPair.into());
    };

    // 获取费用模式
    let fee_mode = get_fee_mode(pool.collect_fee_mode, a_to_b)?;

    // 计算实际输入数量（如果费用在输入上收取）
    let actual_amount_in = if fee_mode.fees_on_input {
        let total_fee = calculate_fee_on_amount_detailed(
            pool,
            token_amount_in,
            current_point,
            false, // has_referral
            false, // has_partner
        )?;
        token_amount_in.checked_sub(total_fee).unwrap_or(0)
    } else {
        token_amount_in
    };

    // 计算交换结果（不含费用）
    let output_amount = if a_to_b {
        get_swap_result_from_a_to_b(pool.sqrt_price, pool.liquidity, actual_amount_in)?
    } else {
        get_swap_result_from_b_to_a(pool.sqrt_price, pool.liquidity, actual_amount_in)?
    };

    // 如果费用在输出上收取，需要扣除费用
    let final_output = if !fee_mode.fees_on_input {
        let total_fee = calculate_fee_on_amount_detailed(
            pool,
            output_amount,
            current_point,
            false, // has_referral
            false, // has_partner
        )?;
        output_amount.checked_sub(total_fee).unwrap_or(0)
    } else {
        output_amount
    };

    Ok(final_output)
}

 

/// 费用模式结构
#[derive(Debug)]
struct FeeMode {
    pub fees_on_input: bool
}

 
/// 获取费用模式 
fn get_fee_mode(collect_fee_mode: u8, a_to_b: bool) -> Result<FeeMode> {
    let (fees_on_input, _) = match (collect_fee_mode, a_to_b) {
        // CollectFeeMode::BothToken = 0
        (0, true) => (false, false),  // A->B: 输出费用，在TokenB上
        (0, false) => (false, true),  // B->A: 输出费用，在TokenA上
        
        // CollectFeeMode::OnlyB = 1  
        (1, true) => (false, false),  // A->B: 输出费用，在TokenB上
        (1, false) => (true, false),  // B->A: 输入费用，在TokenB上
        
        _ => return Err(ErrorCode::InvalidCollectFeeMode.into()),
    };

    Ok(FeeMode {
        fees_on_input,
    })
}

/// 计算详细费用信息 
fn calculate_fee_on_amount_detailed(
    pool: &Dammv2PoolState,
    amount: u64,
    current_point: u64,
    _has_referral: bool,
    _has_partner: bool,
) -> Result<u64> {
    // 获取总交易费率
    let total_fee_numerator = get_total_trading_fee_numerator(
        pool.cliff_fee_numerator,
        pool.fee_scheduler_mode,
        pool.number_of_period,
        pool.period_frequency,
        pool.reduction_factor,
        pool.dynamic_fee_initialized,
        pool.variable_fee_control,
        pool.bin_step,
        pool.volatility_accumulator,
        current_point,
        pool.activation_point,
    )?;

    // 限制最大费率
    let max_fee_numerator = 500_000_000u64; // 50%
    let final_fee_numerator = std::cmp::min(total_fee_numerator, max_fee_numerator);

    // 🔥 关键修复：使用向上舍入，完全按照官方SDK
    // lp_fee = safe_mul_div_cast_u64(amount, trade_fee_numerator, FEE_DENOMINATOR, Rounding::Up)
    let total_fee = safe_mul_div_cast(
        amount as u128,
        final_fee_numerator as u128,
        1_000_000_000u128, // FEE_DENOMINATOR
        Rounding::Up, // 🔥 官方使用向上舍入！
    ) as u64;

    // msg!("calculate_fee_on_amount_detailed total_fee: {}", total_fee);
    Ok(total_fee)
}



/// 获取总交易费率分子
fn get_total_trading_fee_numerator(
    cliff_fee_numerator: u64,
    fee_scheduler_mode: u8,
    number_of_period: u16,
    period_frequency: u64,
    reduction_factor: u64,
    dynamic_fee_initialized: u8,
    variable_fee_control: u32,
    bin_step: u16,
    volatility_accumulator: u128,
    current_point: u64,
    activation_point: u64,
) -> Result<u64> {
    // 1. 计算基础费率
    let base_fee_numerator = get_current_base_fee_numerator(
        cliff_fee_numerator,
        fee_scheduler_mode,
        number_of_period,
        period_frequency,
        reduction_factor,
        current_point,
        activation_point,
    )?;

    // 2. 计算动态费率
    let variable_fee = if dynamic_fee_initialized != 0 {
        get_variable_fee(volatility_accumulator, bin_step, variable_fee_control)? as u64
    } else {
        0
    };

    // 3. 总费率
    let total_fee_numerator = base_fee_numerator.checked_add(variable_fee).unwrap();

    Ok(total_fee_numerator)
}

/// A->B交换计算 
fn get_swap_result_from_a_to_b(
    sqrt_price: u128,
    liquidity: u128,
    amount_in: u64,
) -> Result<u64> {
    // 计算新的目标价格: √P' = √P * L / (L + Δx * √P)
    let next_sqrt_price = get_next_sqrt_price_from_amount_a(sqrt_price, liquidity, amount_in)?;
    
    // 计算输出数量: Δb = L * (√P - √P')
    let output_amount = get_delta_amount_b(next_sqrt_price, sqrt_price, liquidity)?;
    
    Ok(output_amount)
}

/// B->A交换计算 
fn get_swap_result_from_b_to_a(
    sqrt_price: u128,
    liquidity: u128,
    amount_in: u64,
) -> Result<u64> {
    // 计算新的目标价格: √P' = √P + Δy / L
    let next_sqrt_price = get_next_sqrt_price_from_amount_b(sqrt_price, liquidity, amount_in)?;
    
    // 计算输出数量: Δa = L * (1/√P - 1/√P') = L * (√P' - √P) / (√P' * √P)
    let output_amount = get_delta_amount_a(sqrt_price, next_sqrt_price, liquidity)?;
    
    Ok(output_amount)
}

/// 计算下一个sqrt_price (从amount_a) - Always round UP 
/// √P' = √P * L / (L + Δx * √P)
/// Round up to make sure we don't pass the target price
fn get_next_sqrt_price_from_amount_a(
    sqrt_price: u128,
    liquidity: u128,
    amount: u64,
) -> Result<u128> {
    if amount == 0 {
        return Ok(sqrt_price);
    }
    let sqrt_price = U256::from(sqrt_price);
    let liquidity = U256::from(liquidity);

    let product = U256::from(amount).checked_mul(sqrt_price).unwrap();
    let denominator = liquidity.checked_add(U256::from(product)).unwrap();
    let result = mul_div_u256(liquidity, sqrt_price, denominator, Rounding::Up)
        .ok_or_else(|| ErrorCode::MathOverflow)?;
    
    
    Ok(result.try_into().unwrap())
}

/// 计算下一个sqrt_price (从amount_b) - 使用安全的U256计算
/// √P' = √P + Δy / L
/// Round down to minimize price impact
fn get_next_sqrt_price_from_amount_b(
    sqrt_price: u128,
    liquidity: u128,
    amount: u64,
) -> Result<u128> {
    let quotient = U256::from(amount)
        .checked_shl(128)
        .unwrap()
        .checked_div(U256::from(liquidity))
        .unwrap();

    let result = U256::from(sqrt_price).checked_add(quotient).unwrap();

    Ok(result.try_into().unwrap())
}

/// 计算delta_amount_a -  
/// Δa = L * (1/√P_lower - 1/√P_upper) = L * (√P_upper - √P_lower) / (√P_upper * √P_lower)
fn get_delta_amount_a(
    lower_sqrt_price: u128,
    upper_sqrt_price: u128,
    liquidity: u128,
) -> Result<u64> {
    
    // msg!("get_delta_amount_a lower_sqrt_price: {}, upper_sqrt_price: {}, liquidity: {}", lower_sqrt_price, upper_sqrt_price, liquidity);
    let numerator_1 = U256::from(liquidity);
    let numerator_2 = U256::from(upper_sqrt_price - lower_sqrt_price);

    let denominator = U256::from(lower_sqrt_price).checked_mul(U256::from(upper_sqrt_price)).unwrap();

    assert!(denominator > U256::ZERO);
    let result = mul_div_u256(numerator_1, numerator_2, denominator, Rounding::Down)
        .ok_or_else(|| ErrorCode::MathOverflow)?;
 
    require!(result <= U256::from(u64::MAX), ErrorCode::MathOverflow);

    // msg!("get_delta_amount_a result: {:?}", result);

    Ok(result.try_into().unwrap())
}

/// 计算delta_amount_b - 
/// Δb = L * (√P_upper - √P_lower)
fn get_delta_amount_b(
    lower_sqrt_price: u128,
    upper_sqrt_price: u128,
    liquidity: u128,
) -> Result<u64> {

    // msg!("get_delta_amount_b lower_sqrt_price: {}, upper_sqrt_price: {}, liquidity: {}", lower_sqrt_price, upper_sqrt_price, liquidity);
    let liquidity = U256::from(liquidity);
    let delta_sqrt_price = U256::from(upper_sqrt_price - lower_sqrt_price);
    let prod = liquidity.checked_mul(delta_sqrt_price).unwrap();
    
    let (result, _) = prod.overflowing_shr(128);

    require!(result <= U256::from(u64::MAX), ErrorCode::MathOverflow);
    
    // msg!("get_delta_amount_b result: {:?}", result);
    Ok(result.try_into().unwrap())
}