use crate::utils::errors::ErrorCode;
use crate::utils::u128x128_math::{safe_mul_shr_cast, safe_shl_div_cast, Rounding};
use crate::utils::u64x64_math::{pow, ONE, SCALE_OFFSET};
use crate::utils::utils::get_transfer_fee;
use anchor_lang::prelude::*;
 
pub mod dlmm_program_id {
    use super::*;
    declare_id!("LBUZKhRxPF3XUpBCjp4YzTKgLccjZhTSDM9YuVaPwxo");
}

/// Bin Array相关常量
pub const MAX_BIN_PER_ARRAY: usize = 70;
// pub const BIN_ARRAY_BITMAP_SIZE: i32 = 512;

/// DLMM池状态数据结构 (包含费率计算所需的所有字段)
#[derive(Debug, Clone)]
pub struct DlmmPoolState {
    /// DLMM program id index
    pub program_id_index: usize,
    /// DLMM event authority index
    pub event_authority_index: usize,
    /// DLMM oracle index
    pub oracle_index: usize,
    /// DLMM pool index
    pub pool_index: usize,
    /// Token X mint
    pub token_x_mint: Pubkey,
    /// Token Y mint  
    pub token_y_mint: Pubkey,
    /// 活跃bin ID
    pub active_id: i32,
    /// Bin步长
    pub bin_step: u16,

    // StaticParameters中的费率相关字段
    /// 基础费率因子
    pub base_factor: u16,
    /// 可变费率控制参数
    pub variable_fee_control: u32,

    pub max_volatility_accumulator: u32,

    /// 基础费率幂因子
    pub base_fee_power_factor: u8,

    pub total_fee_rate: u64,

    pub base_fee: u64,

    pub price: u128,

    // VariableParameters中的费率相关字段
    /// 波动性累积器
    pub volatility_accumulator: u32,
    /// 波动性参考值
    pub volatility_reference: u32,
    /// 索引参考值
    pub index_reference: i32,
}

/// 单个Bin的状态（简化版本，仅包含必要字段）
#[derive(Debug, Clone, Default)]
pub struct BinState {
    pub amount_x: u64,
    pub amount_y: u64,
    pub price: u128, // Q64.64格式的价格
    pub max_amount_in_with_fees: u64,
}

/// Bin Array状态（简化版本）
// #[derive(Debug, Clone)]
// pub struct BinArrayState {
//     pub index: i32,
//     pub bins: Vec<BinState>, // 最多70个bin
// }

// impl BinArrayState {
//     /// 检查bin_id是否在当前array范围内
//     pub fn is_bin_id_within_range(&self, bin_id: i32) -> bool {
//         let lower_bin_id = self.index * MAX_BIN_PER_ARRAY as i32;
//         let upper_bin_id = lower_bin_id + MAX_BIN_PER_ARRAY as i32 - 1;
//         bin_id >= lower_bin_id && bin_id <= upper_bin_id
//     }

//     /// 获取bin在array中的索引
//     pub fn get_bin_index_in_array(&self, bin_id: i32) -> Option<usize> {
//         if !self.is_bin_id_within_range(bin_id) {
//             return None;
//         }
//         let lower_bin_id = self.index * MAX_BIN_PER_ARRAY as i32;
//         Some((bin_id - lower_bin_id) as usize)
//     }

//     /// 获取bin状态
//     pub fn get_bin(&self, bin_id: i32) -> Option<&BinState> {
//         let index = self.get_bin_index_in_array(bin_id)?;
//         self.bins.get(index)
//     }
// }

// /// 交换结果
// #[derive(Debug)]
// pub struct SwapResult {
//     pub amount_in_with_fees: u64,
//     pub amount_out: u64,
//     pub fee: u64,
// }

/// 解析DLMM池数据 (完整解析所有费率计算必需字段)
pub fn parse_dlmm_pool_data(
    pool_index: &usize,
    wsol_mint: Pubkey,
    token_mint: Pubkey,
    accounts: &[AccountInfo],
    pool_state: &mut DlmmPoolState,
) -> Result<()> {
    let pool_account = &accounts[*pool_index];

    require!(
        *pool_account.owner == dlmm_program_id::ID,
        ErrorCode::InvalidPoolOwner
    );

    let pool_data = pool_account.data.borrow();

    // 跳过账户判别符(8字节)
    // let base_offset = 8;

    // 解析StaticParameters (偏移量8开始，总共32字节)
    let base_factor =
        u16::from_le_bytes(pool_data[8..10].try_into().unwrap());
    let variable_fee_control = u32::from_le_bytes(
        pool_data[16..20]
            .try_into()
            .unwrap(),
    );
    let max_volatility_accumulator = u32::from_le_bytes(
        pool_data[20..24]
            .try_into()
            .unwrap(),
    );

    let base_fee_power_factor = pool_data[34];

    // 解析VariableParameters (偏移量40开始，总共32字节)
    // let var_offset = 40; // 8 + 32 = 40
    let volatility_accumulator =
        u32::from_le_bytes(pool_data[40..44].try_into().unwrap());

    let volatility_reference = u32::from_le_bytes(
        pool_data[44..48]
            .try_into()
            .unwrap(),
    );
    let index_reference = i32::from_le_bytes(
        pool_data[48..52]
            .try_into()
            .unwrap(),
    );
    // 解析LbPair主体字段 (偏移量72开始)
    // let pair_offset = 72; // 40 + 32 = 72

    let active_id = i32::from_le_bytes(
        pool_data[76..80]
            .try_into()
            .unwrap(),
    );
    let bin_step = u16::from_le_bytes(
        pool_data[80..82]
            .try_into()
            .unwrap(),
    );

    // tokenXMint在72 + 16 = 88字节处开始
    let token_x_mint = Pubkey::try_from(&pool_data[88..120]).unwrap();
    // tokenYMint在88 + 32 = 120字节处开始
    let token_y_mint = Pubkey::try_from(&pool_data[120..152]).unwrap();

    // 验证池子包含SOL和指定token的配对
    let (_, other_mint) = if token_x_mint == wsol_mint {
        (token_x_mint, token_y_mint)
    } else if token_y_mint == wsol_mint {
        (token_y_mint, token_x_mint)
    } else {
        return Err(ErrorCode::InvalidTokenPair.into());
    };

    require!(other_mint == token_mint, ErrorCode::TokenMismatch);
   
    drop(pool_data);
    
    // reserve地址在token_y_mint之后
    // let reserve_x = Pubkey::try_from(&pool_data[pair_offset + 80..pair_offset + 112]).unwrap();
    // let reserve_y = Pubkey::try_from(&pool_data[pair_offset + 112..pair_offset + 144]).unwrap();
    pool_state.token_x_mint = token_x_mint;
    pool_state.token_y_mint = token_y_mint;
    pool_state.active_id = active_id;
    pool_state.bin_step = bin_step;
    pool_state.base_factor = base_factor;
    pool_state.variable_fee_control = variable_fee_control;
    pool_state.max_volatility_accumulator = max_volatility_accumulator;
    pool_state.base_fee_power_factor = base_fee_power_factor;
    pool_state.volatility_accumulator = volatility_accumulator;
    pool_state.volatility_reference = volatility_reference;
    pool_state.index_reference = index_reference;

    Ok(())
}

/// 计算DLMM价格 (返回Q64.64格式)
pub fn calculate_dlmm_price(pool: &DlmmPoolState, wsol_mint: Pubkey) -> Result<u128> {
    // 计算bin_step的Q64.64表示: bin_step / 10000
    let bps = u128::from(pool.bin_step)
        .checked_shl(SCALE_OFFSET.into())
        .unwrap()
        .checked_div(10000u128)
        .unwrap();

    // 计算(1 + bin_step)的Q64.64表示
    let base = ONE.checked_add(bps).unwrap();

    // 计算(1 + bin_step)^active_id
    let mut price_q64 = pow(base, pool.active_id).unwrap();

    // 如果WSOL不是TokenY，需要取倒数
    // 在Q64.64格式中，倒数 = (2^64)^2 / price = 2^128 / price
    if pool.token_y_mint != wsol_mint {
        if price_q64 == 0 {
            return Err(ErrorCode::ZeroPrice.into());
        }
        // Q64.64倒数计算
        price_q64 = (ONE.checked_shl(SCALE_OFFSET.into()).unwrap())
            .checked_div(price_q64)
            .unwrap();
    }

    // 转换为f64用于打印 (price / 2^64)
    let price_f64 = price_q64 as f64 / ONE as f64;
    msg!("DLMM价格: {:.8} SOL", price_f64);

    // 返回完整的Q64.64格式
    Ok(price_q64)
}

// // 快速计算DLMM价格 (用于价格比较阶段，不需要bin array数据)
// // 只基于active bin价格，不考虑流动性分布
// pub fn calculate_dlmm_price_fast(
//     pool: &DlmmPoolState,
//     wsol_mint: Pubkey,
// ) -> Result<u128> {
//     // 直接使用active bin的价格
//     let mut price_q64 = calculate_bin_price(pool.active_id, pool.bin_step)?;

//     // 如果WSOL不是TokenY，需要取倒数
//     if pool.token_y_mint != wsol_mint {
//         if price_q64 == 0 {
//             return Err(ErrorCode::ZeroPrice.into());
//         }
//         price_q64 = (ONE.checked_shl(SCALE_OFFSET.into()).unwrap())
//             .checked_div(price_q64)
//             .unwrap();
//     }

//     Ok(price_q64)
// }

/// 🚀 超高效价格读取 - 直接从bin array中读取active bin的价格
/// 比计算价格节省大量CU，因为bin中的price字段已经是Q64.64格式
pub fn get_dlmm_price_from_bin_array(
    wsol_mint: Pubkey,
    // accounts: &[AccountInfo],
    // bin_array_0_index: usize,
    pool_state: &mut DlmmPoolState,
) -> Result<()> {
    // 确定active_id在哪个bin array中
    let active_id = pool_state.active_id;
    // 根据bin_array_index选择对应的账户
    // let bin_array_account = &accounts[bin_array_0_index];

    //  // 直接读取active bin的价格
    //  let mut price_q64 = read_bin_price_from_array(bin_array_account, active_id)?;
    // 🔥 关键修复：使用计算的价格而不是读取的价格
    let mut price_q64 = calculate_bin_price(active_id, pool_state.bin_step)?;

    if price_q64 == 0 {
        return Err(ErrorCode::ZeroPrice.into());
    }

    // 如果WSOL不是TokenY，需要取倒数
    if pool_state.token_y_mint != wsol_mint {
        price_q64 = (ONE.checked_shl(SCALE_OFFSET.into()).unwrap())
            .checked_div(price_q64)
            .unwrap();
    }

    pool_state.price = price_q64;

    Ok(())
}

/// 更新波动性累积器
pub fn calculate_volatility_accumulator(pool_state: &DlmmPoolState, active_id: i32) -> Result<u32> {
    let delta_id = i64::from(pool_state.index_reference)
        .checked_sub(active_id.into())
        .unwrap()
        .unsigned_abs();

    let volatility_accumulator = u64::from(pool_state.volatility_reference)
        .checked_add(delta_id.checked_mul(10_000).unwrap())
        .unwrap();

    let volatility_accumulator = std::cmp::min(
        volatility_accumulator,
        pool_state.max_volatility_accumulator.into(),
    )
    .try_into()
    .unwrap();

    Ok(volatility_accumulator)
}

// /// 从bin array中读取指定bin_id的价格
// /// 直接定位到bin位置，读取Q64.64格式的价格
// fn read_bin_price_from_array(bin_array_account: &AccountInfo, bin_id: i32) -> Result<u128> {
//     require!(
//         bin_array_account.owner == &dlmm_program_id::ID,
//         ErrorCode::InvalidPoolOwner
//     );

//     let data = bin_array_account.data.borrow();

//     // 解析bin array index
//     let mut offset = 8;
//     let index_i64 = i64::from_le_bytes(data[offset..offset + 8].try_into().unwrap());
//     let index = index_i64 as i32;

//     // 计算bin在array中的位置
//     let start_bin_id = index * MAX_BIN_PER_ARRAY as i32;
//     let end_bin_id = start_bin_id + MAX_BIN_PER_ARRAY as i32 - 1;

//     // 检查bin_id是否在当前array范围内
//     if bin_id < start_bin_id || bin_id > end_bin_id {
//         msg!(
//             "bin_id: {} is out of range: {}",
//             bin_id,
//             bin_array_account.key
//         );
//         return Err(ErrorCode::BinIdOutOfRange.into());
//     }

//     // 计算bin在array中的索引
//     let bin_index = (bin_id - start_bin_id) as usize;

//     // 跳过header: index(8) + version(1) + padding(7) + lbPair(32) = 48字节
//     offset = 8 + 48;

//     // 每个bin占144字节，price字段在偏移量16处，占16字节
//     let bin_offset = offset + bin_index * 144;
//     let price_offset = bin_offset + 16; // amount_x(8) + amount_y(8) = 16

//     // 读取Q64.64格式的价格
//     let price_bytes = &data[price_offset..price_offset + 16];
//     let price = u128::from_le_bytes(price_bytes.try_into().unwrap());

//     Ok(price)
// }

/// 计算DLMM总费率 (完全按照SDK实现，不做任何假设)
pub fn calculate_dlmm_total_fee_rate(pool_state: &mut DlmmPoolState) -> Result<()> {
    // 1. 计算基础费率 (完全按照SDK公式)
    // base_fee = base_factor * bin_step * 10 * 10^base_fee_power_factor
    let base_fee = u128::from(pool_state.base_factor)
        .checked_mul(pool_state.bin_step as u128)
        .unwrap()
        .checked_mul(10u128)
        .unwrap()
        .checked_mul(10u128.pow(pool_state.base_fee_power_factor as u32))
        .unwrap();

    // 2. 计算可变费率 (完全按照SDK公式，包含向上取整)
    let variable_fee = if pool_state.variable_fee_control > 0 {
        // variable_fee = variable_fee_control * (volatility_accumulator * bin_step)^2
        let square_vfa_bin = u128::from(pool_state.volatility_accumulator)
            .checked_mul(pool_state.bin_step as u128)
            .unwrap()
            .checked_pow(2)
            .unwrap();

        let v_fee = u128::from(pool_state.variable_fee_control)
            .checked_mul(square_vfa_bin)
            .unwrap();

        // 向上取整除法: (v_fee + 99_999_999_999) / 100_000_000_000
        let scaled_v_fee = v_fee
            .checked_add(99_999_999_999u128)
            .unwrap()
            .checked_div(100_000_000_000u128)
            .unwrap();

        scaled_v_fee
    } else {
        0
    };

    // 3. 计算总费率
    let total_fee = base_fee.checked_add(variable_fee).unwrap();

    let total_fee = std::cmp::min(total_fee, 100_000_000u128) as u128;

    // // 4. 转换为基点(bps) - 从FEE_PRECISION转换为BASE_BPS
    // let final_fee_bps = total_fee
    //     .checked_mul(BASE_BPS as u128)
    //     .unwrap()
    //     .checked_div(1_000_000_000u128)
    //     .unwrap();
    // msg!("final_fee_bps: {}", final_fee_bps);

    pool_state.base_fee = base_fee as u64;
    pool_state.total_fee_rate = total_fee as u64;

    Ok(())
}

/// 🚀 优化版：计算动态总费率，避免重复代码
fn calculate_dynamic_total_fee_rate(pool: &DlmmPoolState, active_id: i32) -> u64 {
    if pool.variable_fee_control == 0 {
        return pool.total_fee_rate;
    }

    let volatility_accumulator = calculate_volatility_accumulator(pool, active_id).unwrap();

    // 如果波动性累积器没有变化，直接使用缓存的费率
    if volatility_accumulator == pool.volatility_accumulator {
        return pool.total_fee_rate;
    }

    // 重新计算可变费率
    let square_vfa_bin = u128::from(volatility_accumulator)
        .checked_mul(pool.bin_step as u128)
        .unwrap()
        .checked_pow(2)
        .unwrap();

    let v_fee = u128::from(pool.variable_fee_control)
        .checked_mul(square_vfa_bin)
        .unwrap();

    // 向上取整除法
    let variable_fee = v_fee
        .checked_add(99_999_999_999u128)
        .unwrap()
        .checked_div(100_000_000_000u128)
        .unwrap();

    // 计算总费率并限制上限
    let total_fee = pool.base_fee.checked_add(variable_fee as u64).unwrap();
    std::cmp::min(total_fee, 100_000_000u64)
}

/// 🚀 优化版：根据输入金额计算手续费
pub fn compute_fee(pool: &DlmmPoolState, amount: u64, active_id: i32) -> u64 {
    let total_fee_rate = calculate_dynamic_total_fee_rate(pool, active_id);

    let denominator = 1_000_000_000u128
        .checked_sub(total_fee_rate as u128)
        .unwrap();

    // 向上取整除法
    let fee = u128::from(amount)
        .checked_mul(total_fee_rate as u128)
        .unwrap()
        .checked_add(denominator)
        .unwrap()
        .checked_sub(1)
        .unwrap();

    fee.checked_div(denominator).unwrap() as u64
}

/// 🚀 优化版：从含手续费金额中计算手续费
pub fn compute_fee_from_amount(pool: &DlmmPoolState, amount_with_fees: u64, active_id: i32) -> u64 {
    let total_fee_rate = calculate_dynamic_total_fee_rate(pool, active_id);

    let fee_amount = u128::from(amount_with_fees)
        .checked_mul(total_fee_rate as u128)
        .unwrap()
        .checked_add(999_999_999)
        .unwrap();

    fee_amount.checked_div(1_000_000_000).unwrap() as u64
}

/// 使用预处理的有序bins
pub fn dlmm_quote_exact_input_wsol_optimized(
    pool: &DlmmPoolState,
    wsol_mint: Pubkey,
    wsol_amount_in: u64,
    ordered_bins: &[(i32, BinState)], // 已按交易方向排序的bins
    token_mint_info: &AccountInfo,
) -> Result<u64> {
    // 确定交易方向：WSOL输入，Token输出
    let swap_for_y = if pool.token_x_mint == wsol_mint {
        true // WSOL是X输入，Token是Y输出，输出Y，所以swap_for_y=true
    } else if pool.token_y_mint == wsol_mint {
        false // WSOL是Y输入，Token是X输出，输出X，所以swap_for_y=false
    } else {
        return Err(ErrorCode::InvalidTokenPair.into());
    };

    let token_amount_out = calculate_exact_output_across_bins_optimized(
        pool,
        wsol_amount_in,
        swap_for_y, // 输入wsol，输出token
        ordered_bins,
    )?;

    //check token 2022 fee
    let transfer_fee = get_transfer_fee(token_mint_info, token_amount_out)?;
    let token_amount_out = match transfer_fee {
        0 => token_amount_out,
        _ => token_amount_out.checked_sub(transfer_fee).unwrap(),
    };

    Ok(token_amount_out)
}

/// 🚀 DLMM超级优化Quote函数 - Token输入版本
pub fn dlmm_quote_exact_input_token_optimized(
    pool_state: &DlmmPoolState,
    wsol_mint: Pubkey,
    token_amount_in: u64,
    ordered_bins: &[(i32, BinState)], // 已按交易方向排序的bins
    token_mint_info: &AccountInfo,
) -> Result<u64> {
    // 确定交易方向：Token输入，WSOL输出
    let swap_for_y = if pool_state.token_x_mint == wsol_mint {
        false // Token是Y输入，WSOL是X输出，输出X，所以swap_for_y=false
    } else if pool_state.token_y_mint == wsol_mint {
        true // Token是X输入，WSOL是Y输出，输出Y，所以swap_for_y=true
    } else {
        return Err(ErrorCode::InvalidTokenPair.into());
    };

    //check token 2022 fee
    let transfer_fee = get_transfer_fee(token_mint_info, token_amount_in)?;
    let token_amount_in = match transfer_fee {
        0 => token_amount_in,
        _ => token_amount_in.checked_sub(transfer_fee).unwrap(),
    };

    // 🚀 使用超级优化的跨bin计算
    let wsol_amount_out = calculate_exact_output_across_bins_optimized(
        pool_state,
        token_amount_in,
        swap_for_y, // 输入token，输出wsol
        ordered_bins,
    )?;

    Ok(wsol_amount_out)
}

/// 🚀 超级优化版本：使用预处理的有序bins进行跨bin计算
/// 无需边界检查、无需bin查找、无需bins_processed计数
fn calculate_exact_output_across_bins_optimized(
    pool: &DlmmPoolState,
    amount_in: u64,
    swap_for_y: bool,
    ordered_bins: &[(i32, BinState)], // 已按交易方向排序的bins
) -> Result<u64> {
    let mut remaining_amount_in = amount_in;
    let mut total_amount_out = 0u64;

    // 🔥 关键优化：直接遍历有序的bins，无需任何查找和边界检查
    for &(bin_id, ref bin_state) in ordered_bins {
        if remaining_amount_in == 0 {
            break;
        }

        // 计算当前bin中的交换结果
        let (amount_in_consumed, amount_out_produced) =
            calculate_swap_in_bin(pool, remaining_amount_in, bin_state, swap_for_y, bin_id)?;

        // 更新累计结果
        remaining_amount_in = remaining_amount_in
            .checked_sub(amount_in_consumed)
            .unwrap_or(0);
        total_amount_out = total_amount_out
            .checked_add(amount_out_produced)
            .unwrap_or(0);
    }

    Ok(total_amount_out)
}

/// 计算单个bin中的交换结果 
fn calculate_swap_in_bin(
    pool: &DlmmPoolState,
    amount_in: u64,
    bin_state: &BinState,
    swap_for_y: bool,
    bin_id: i32,
) -> Result<(u64, u64)> {
    if amount_in == 0 {
        return Ok((0, 0));
    }

    let max_amount_out = get_max_amount_out(bin_state.amount_y, bin_state.amount_x, swap_for_y);
    let max_amount_in = bin_state.max_amount_in_with_fees;

    let (amount_in_with_fees, amount_out) = if amount_in > max_amount_in {
        (max_amount_in, max_amount_out)
    } else {
        let fee = compute_fee_from_amount(pool, amount_in, bin_id);
        let amount_in_after_fee = amount_in.checked_sub(fee).unwrap();
        let amount_out = get_amount_out(amount_in_after_fee, bin_state.price, swap_for_y);
        (amount_in, std::cmp::min(amount_out, max_amount_out))
    };

    Ok((amount_in_with_fees, amount_out))
}

/// 直接在解析过程中根据价格判断停止条件，无需预计算bin_id范围
pub fn parse_dlmm_bin_arrays_optimized(
    pool: &DlmmPoolState,
    accounts: &[AccountInfo],
    bin_array_minus_1_index: usize,
    bin_array_0_index: usize,
    bin_array_1_index: usize,
    active_id: i32,
    swap_for_y: bool,
    buy_pool_price: u128,  // 买入池价格 (Q64.64格式)
    sell_pool_price: u128, // 卖出池价格 (Q64.64格式)
    wsol_mint: Pubkey,
    is_buy: bool,
) -> Result<Vec<(i32, BinState)>> {
    let mut all_relevant_bins = Vec::with_capacity(30);

    // 🚀 关键优化：确定价格停止条件
    let price_limit = if is_buy {
        // X -> Y: 价格递减
        sell_pool_price
    } else {
        // Y -> X: 价格递增
        buy_pool_price
    };

    let mut reached_limit = false;

    // 🚀 智能检测 active_id 实际在哪个 bin array 中
    let all_bin_array_indices = [
        bin_array_minus_1_index,
        bin_array_0_index,
        bin_array_1_index,
    ];
    let mut active_array_found_index: Option<usize> = None;

    let bin_array_account_0 = &accounts[bin_array_0_index];
    if let Some(array_range) = get_bin_array_range(bin_array_account_0)? {
        if active_id >= array_range.0 && active_id <= array_range.1 {
            active_array_found_index = Some(1);
        } else {
            // 快速检测 active_id 在哪个 array 中（最多检查3个array）
            for (i, &array_index) in all_bin_array_indices.iter().enumerate() {
                if i == 1 {
                    continue;
                }
                let bin_array_account = &accounts[array_index];
                if let Some(array_range) = get_bin_array_range(bin_array_account)? {
                    if active_id >= array_range.0 && active_id <= array_range.1 {
                        active_array_found_index = Some(i);
                        break;
                    }
                }
            }
        }
    }

    // 🚀 如果找到了 active_id 所在的 array，从那里开始按方向解析
    if let Some(start_index) = active_array_found_index {
        let parsing_order = if swap_for_y {
            // X->Y: 从 active array 向左解析 (向 minus_1 方向)
            (start_index..=2).rev().collect::<Vec<_>>() // 从找到的位置向左
        } else {
            // Y->X: 从 active array 向右解析 (向 +1 方向)
            (start_index..3).collect::<Vec<_>>() // 从找到的位置向右
        };

        // 按确定的顺序解析 bin arrays
        for &order_index in &parsing_order {
            if reached_limit {
                break;
            }

            let array_index = all_bin_array_indices[order_index];
            let bin_array_account = &accounts[array_index];

            reached_limit = parse_bin_array_data_selective(
                pool,
                bin_array_account,
                active_id,
                swap_for_y,
                price_limit,
                &mut all_relevant_bins,
                wsol_mint,
            )?;
 
        }
    } else {
        msg!(
            "Warning: active_id {} not found in any provided bin arrays",
            active_id
        );
    }

    // msg!("all_relevant_bins length: {}, pool_index: {}, active_id: {}", all_relevant_bins.len() , pool.pool_index, active_id);
    // msg!("swap_for_y: {}", swap_for_y);
    // msg!("price_limit_len: {} ", price_limit_len);
    // msg!("all_relevant_bins length: {} ", all_relevant_bins.len());

    Ok(all_relevant_bins)
}

/// 🚀 优化版：选择性解析单个bin array，智能处理active_id跨array情况
/// 返回 (是否找到有效bins, 是否达到价格限制)
fn parse_bin_array_data_selective(
    pool: &DlmmPoolState,
    bin_array_account: &AccountInfo,
    active_id: i32,
    swap_for_y: bool,
    price_limit: u128,
    all_relevant_bins: &mut Vec<(i32, BinState)>,
    wsol_mint: Pubkey,
) -> Result<bool> {
    // 🛡️ 检查bin array是否已初始化
    if bin_array_account.owner != &dlmm_program_id::ID {
        msg!(
            "Skipping uninitialized bin_array: {:?}",
            bin_array_account.key.to_string()
        );
        return Ok(false); // 未初始化的array直接跳过，不影响后续处理
    }

    let data = bin_array_account.data.borrow();

    let mut offset = 8;
    let index_i64 = i64::from_le_bytes(data[offset..offset + 8].try_into().unwrap());
    let index = index_i64 as i32;
    offset += 48; // 跳过 index(8) + version(1) + padding(7) + lbPair(32)

    let start_bin_id = index * MAX_BIN_PER_ARRAY as i32;
    let end_bin_id = start_bin_id + MAX_BIN_PER_ARRAY as i32 - 1;

    // 🚀 智能判断循环范围：处理active_id可能在相邻array的情况
    let is_active_array = active_id >= start_bin_id && active_id <= end_bin_id;
    let mut reached_limit = false;

    // 🚀 根据交易方向和active_id位置智能确定循环范围
    if swap_for_y {
        // X -> Y: 价格递减，需要向左搜索
        let loop_range: Box<dyn Iterator<Item = usize>> = if is_active_array {
            // active_id在当前array中，从active_id开始向左
            let start_idx = (active_id - start_bin_id) as usize;
            Box::new((0..=start_idx).rev())
        } else if active_id > end_bin_id {
            // active_id在右侧array中，当前array全部遍历
            Box::new((0..MAX_BIN_PER_ARRAY).rev())
        } else {
            // active_id在左侧array中，跳过当前array
            return Ok(false);
        };

        for i in loop_range {
            let bin_id = start_bin_id + i as i32;
            let bin_offset = offset + i * 144;

            let amount_x = u64::from_le_bytes(data[bin_offset..bin_offset + 8].try_into().unwrap());
            let amount_y =
                u64::from_le_bytes(data[bin_offset + 8..bin_offset + 16].try_into().unwrap());

            // 只保存有Y流动性的bin
            if amount_y > 0 {
                // msg!("找到Y流动性bin: bin_id={}, amount_y={} active_id={}", bin_id, amount_y, active_id);
                // let price_bytes = &data[bin_offset + 16..bin_offset + 32];
                // let mut price = u128::from_le_bytes(price_bytes.try_into().unwrap());
                // 🔥 关键修复：使用计算的价格而不是读取的价格
                let mut price = calculate_bin_price(bin_id, pool.bin_step)?;

                if pool.token_y_mint != wsol_mint {
                    price = (ONE.checked_shl(SCALE_OFFSET.into()).unwrap())
                        .checked_div(price)
                        .unwrap();
                }

                // 价格小于限制 直接停止
                if price < price_limit || all_relevant_bins.len() >= 30 {
                    reached_limit = true;
                    break;
                }
 

                let max_amount_in = get_max_amount_in(amount_y, amount_x, price, swap_for_y);
                let max_fee = compute_fee(pool, max_amount_in, bin_id);
                let max_amount_in_with_fees = max_amount_in.checked_add(max_fee).unwrap();

                all_relevant_bins.push((
                    bin_id,
                    BinState {
                        amount_x,
                        amount_y,
                        price,
                        max_amount_in_with_fees,
                    },
                ));
            }
        }
    } else {
        // Y -> X: 价格递增，需要向右搜索
        let loop_range: Box<dyn Iterator<Item = usize>> = if is_active_array {
            // active_id在当前array中，从active_id开始向右
            let start_idx = (active_id - start_bin_id) as usize;
            Box::new(start_idx..MAX_BIN_PER_ARRAY)
        } else if active_id < start_bin_id {
            // active_id在左侧array中，当前array全部遍历
            Box::new(0..MAX_BIN_PER_ARRAY)
        } else {
            // active_id在右侧array中，跳过当前array
            return Ok(false);
        };

        for i in loop_range {
            let bin_id = start_bin_id + i as i32;
            let bin_offset = offset + i * 144;

            let amount_x = u64::from_le_bytes(data[bin_offset..bin_offset + 8].try_into().unwrap());
            let amount_y =
                u64::from_le_bytes(data[bin_offset + 8..bin_offset + 16].try_into().unwrap());

            // 只保存有X流动性的bin
            if amount_x > 0 {
                // msg!("找到X流动性bin: bin_id={}, amount_x={} active_id={}", bin_id, amount_x, active_id);
                // let price_bytes = &data[bin_offset + 16..bin_offset + 32];
                // let mut price = u128::from_le_bytes(price_bytes.try_into().unwrap());
                // 🔥 关键修复：使用计算的价格而不是读取的价格
                let mut price = calculate_bin_price(bin_id, pool.bin_step)?;

                if pool.token_y_mint != wsol_mint {
                    price = (ONE.checked_shl(SCALE_OFFSET.into()).unwrap())
                        .checked_div(price)
                        .unwrap();
                }

                // 价格大于限制 直接停止
                if price > price_limit || all_relevant_bins.len() >= 30 {
                    reached_limit = true;
                    break;
                }
 

                let max_amount_in = get_max_amount_in(amount_y, amount_x, price, swap_for_y);
                let max_fee = compute_fee(pool, max_amount_in, bin_id);
                let max_amount_in_with_fees = max_amount_in.checked_add(max_fee).unwrap();

                all_relevant_bins.push((
                    bin_id,
                    BinState {
                        amount_x,
                        amount_y,
                        price,
                        max_amount_in_with_fees,
                    },
                ));
            }
        }
    }

    Ok(reached_limit)
}

/// 计算指定bin ID的价格
fn calculate_bin_price(bin_id: i32, bin_step: u16) -> Result<u128> {
    // 计算bin_step的Q64.64表示: bin_step / 10000
    let bps = u128::from(bin_step)
        .checked_shl(SCALE_OFFSET.into())
        .unwrap()
        .checked_div(10000u128)
        .unwrap();

    // 计算(1 + bin_step)的Q64.64表示
    let base = ONE.checked_add(bps).unwrap();

    // 计算(1 + bin_step)^bin_id
    pow(base, bin_id).ok_or(ErrorCode::MathOverflow.into())
}

fn get_max_amount_out(amount_y: u64, amount_x: u64, swap_for_y: bool) -> u64 {
    if swap_for_y {
        amount_y
    } else {
        amount_x
    }
}

fn get_max_amount_in(amount_y: u64, amount_x: u64, price: u128, swap_for_y: bool) -> u64 {
    if swap_for_y {
        safe_shl_div_cast(amount_y.into(), price, SCALE_OFFSET, Rounding::Up) as u64
    } else {
        safe_mul_shr_cast(amount_x.into(), price, SCALE_OFFSET, Rounding::Up) as u64
    }
}

// fn get_amount_in(amount_out: u64, price: u128, swap_for_y: bool) -> u64 {
//     if swap_for_y {
//         safe_shl_div_cast(amount_out.into(), price, SCALE_OFFSET, Rounding::Up) as u64
//     } else {
//         safe_mul_shr_cast(amount_out.into(), price, SCALE_OFFSET, Rounding::Up) as u64
//     }
// }

fn get_amount_out(amount_in: u64, price: u128, swap_for_y: bool) -> u64 {
    if swap_for_y {
        safe_mul_shr_cast(price, amount_in.into(), SCALE_OFFSET, Rounding::Down) as u64
    } else {
        safe_shl_div_cast(amount_in.into(), price, SCALE_OFFSET, Rounding::Down) as u64
    }
}

/// 🚀 快速获取 bin array 的 ID 范围，如果未初始化返回 None
/// 返回 (start_bin_id, end_bin_id)
fn get_bin_array_range(bin_array_account: &AccountInfo) -> Result<Option<(i32, i32)>> {
    // 检查是否已初始化
    if bin_array_account.owner != &dlmm_program_id::ID {
        return Ok(None);
    }

    // let data = bin_array_account.data.borrow();
    // if data.len() < 16 {
    //     return Ok(None);
    // }

    // 读取 index (offset 8-16)
    // let index_i64 = i64::from_le_bytes(data[8..16].try_into().unwrap());
    // let index = index_i64 as i32;

    let index = {
        let data = bin_array_account.data.borrow();
        if data.len() < 16 {
            return Ok(None);
        }
        i64::from_le_bytes(data[8..16].try_into().unwrap()) as i32
    };

    let start_bin_id = index * MAX_BIN_PER_ARRAY as i32;
    let end_bin_id = start_bin_id + MAX_BIN_PER_ARRAY as i32 - 1;

    Ok(Some((start_bin_id, end_bin_id)))
}

