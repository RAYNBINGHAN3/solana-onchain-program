use crate::utils::errors::ErrorCode;
use crate::utils::u128x128_math::{safe_mul_div_cast, safe_mul_shr_cast, Rounding};
use crate::utils::u64x64_math::ONE;
use crate::utils::utils::get_transfer_fee;
use crate::utils::whirlpool_256_math::{
    get_amount_delta_a, get_amount_delta_b, try_get_amount_delta_a, try_get_amount_delta_b,
    get_next_sqrt_price, AmountDeltaU64, FEE_RATE_MUL_VALUE, MAX_SQRT_PRICE_X64, MIN_SQRT_PRICE_X64
};
use anchor_lang::prelude::*;
use std::cmp::min;
use bytemuck::{Pod, Zeroable};

pub mod whirlpool_program_id {
    use super::*;
    declare_id!("whirLbMiicVdio4qvUfM5KAg6Ct8VwpYzGff3uctyCc");
}

/// Whirlpool池状态数据结构 - 与其他DEX风格统一
#[derive(Debug, Clone)]
pub struct WhirlpoolPoolState {
    pub program_id_index: usize,
    pub pool_index: usize,
    pub oracle_index: usize,
    pub vault_a_index: usize,
    pub vault_b_index: usize,
    pub tick_array_0_index: usize,
    pub tick_array_1_index: usize,
    pub tick_array_2_index: usize,

    pub token_mint_a: Pubkey,
    pub token_mint_b: Pubkey,

    pub sqrt_price: u128,
    pub tick_spacing: u16,
    pub tick_current_index: i32,
    pub liquidity: u128,

    pub trade_fee_rate: u64,
    pub price: u128,
}

/// Whirlpool TickArray 类型枚举
#[derive(Debug, Clone, Copy)]
pub enum TickArrayType {
    Fixed,
    Dynamic,
}

/// Whirlpool TickArray - 统一接口支持Fixed和Dynamic两种格式
pub struct WhirlpoolTickArrayRef<'a> {
    account_info: &'a AccountInfo<'a>,
    array_type: TickArrayType,
}

/// Dynamic Tick Array 的 discriminator - 从官方SDK复制
pub const DYNAMIC_TICK_ARRAY_DISCRIMINATOR: [u8; 8] = [17, 216, 246, 142, 225, 199, 218, 56];
/// Fixed Tick Array 的 discriminator - 从官方SDK复制  
pub const FIXED_TICK_ARRAY_DISCRIMINATOR: [u8; 8] = [42, 50, 55, 254, 208, 36, 111, 186];

/// Dynamic Tick 枚举 - 完全按照官方SDK定义
#[derive(Debug, Clone, Copy, Default)]
pub enum DynamicTick {
    #[default]
    Uninitialized,
    Initialized(DynamicTickData),
}

/// Dynamic Tick Data - 完全按照官方SDK定义
#[derive(Debug, Clone, Copy, Default)]
pub struct DynamicTickData {
    pub liquidity_net: i128,
    pub liquidity_gross: u128,
    pub fee_growth_outside_a: u128,
    pub fee_growth_outside_b: u128,
    pub reward_growths_outside: [u128; NUM_REWARDS],
}

impl DynamicTick {
    /// 🎯 完全按照官方SDK定义的常量
    pub const UNINITIALIZED_LEN: usize = 1;
    pub const INITIALIZED_LEN: usize = 113; // 1 (variant) + 112 (DynamicTickData::LEN)
    
    /// 转换为标准Tick格式
    pub fn to_tick(self) -> WhirlpoolTick {
        match self {
            DynamicTick::Uninitialized => WhirlpoolTick::default(),
            DynamicTick::Initialized(data) => WhirlpoolTick {
                initialized: 1,
                liquidity_net: data.liquidity_net,
                liquidity_gross: data.liquidity_gross,
                fee_growth_outside_a: data.fee_growth_outside_a,
                fee_growth_outside_b: data.fee_growth_outside_b,
                reward_growths_outside: data.reward_growths_outside,
            },
        }
    }
}

impl<'a> WhirlpoolTickArrayRef<'a> {
    /// 创建新的TickArray引用 - 自动检测类型
    pub fn new(account_info: &'a AccountInfo<'a>) -> Result<Self> {
        let array_type = Self::detect_array_type(account_info)?;
        Ok(Self { account_info, array_type })
    }
    
    /// 检测TickArray类型 - 通过discriminator
    fn detect_array_type(account_info: &AccountInfo) -> Result<TickArrayType> {
        let data = account_info.try_borrow_data()?;
        if data.len() < 8 {
            return Err(ErrorCode::InvalidAccountData.into());
        }
        
        let discriminator: [u8; 8] = data[0..8].try_into().unwrap();
        match discriminator {
            FIXED_TICK_ARRAY_DISCRIMINATOR => Ok(TickArrayType::Fixed),
            DYNAMIC_TICK_ARRAY_DISCRIMINATOR => Ok(TickArrayType::Dynamic),
            _ => {
                msg!("Unknown tick array discriminator: {:?}", discriminator);
                Err(ErrorCode::InvalidAccountData.into())
            }
        }
    }
    
    /// 获取start_tick_index - 支持两种格式
    pub fn start_tick_index(&self) -> i32 {
        let data = self.account_info.try_borrow_data().unwrap();
        match self.array_type {
            TickArrayType::Fixed => {
                // Fixed: discriminator(8) + start_tick_index(4)
                i32::from_le_bytes(data[8..12].try_into().unwrap())
            }
            TickArrayType::Dynamic => {
                // Dynamic: discriminator(8) + start_tick_index(4)
                i32::from_le_bytes(data[8..12].try_into().unwrap())
            }
        }
    }
    
    /// 获取whirlpool地址 - 支持两种格式
    pub fn whirlpool(&self) -> Pubkey {
        let data = self.account_info.try_borrow_data().unwrap();
        match self.array_type {
            TickArrayType::Fixed => {
                // Fixed: discriminator(8) + start_tick_index(4) + ticks(9944) + whirlpool(32)
                let offset = 8 + 4 + TICK_ARRAY_SIZE * std::mem::size_of::<WhirlpoolTick>();
                Pubkey::try_from(&data[offset..offset + 32]).unwrap()
            }
            TickArrayType::Dynamic => {
                // Dynamic: discriminator(8) + start_tick_index(4) + whirlpool(32)
                Pubkey::try_from(&data[12..44]).unwrap()
            }
        }
    }
    
    /// 获取tick bitmap (仅Dynamic使用)
    fn get_tick_bitmap(&self) -> Option<u128> {
        match self.array_type {
            TickArrayType::Fixed => None,
            TickArrayType::Dynamic => {
                let data = self.account_info.try_borrow_data().unwrap();
                // Dynamic: discriminator(8) + start_tick_index(4) + whirlpool(32) + tick_bitmap(16)
                Some(u128::from_le_bytes(data[44..60].try_into().unwrap()))
            }
        }
    }
    
    /// 获取指定偏移的tick - 支持两种格式
    pub fn get_tick_by_offset(&self, offset: usize) -> Result<WhirlpoolTick> {
        if offset >= TICK_ARRAY_SIZE {
            return Err(ErrorCode::InvalidTickIndex.into());
        }
        
        let data = self.account_info.try_borrow_data()?;
        match self.array_type {
            TickArrayType::Fixed => {
                // Fixed: discriminator(8) + start_tick_index(4) + ticks[offset]
                let tick_offset = 8 + 4 + offset * std::mem::size_of::<WhirlpoolTick>();
                let tick_data = &data[tick_offset..tick_offset + std::mem::size_of::<WhirlpoolTick>()];
                let tick: &WhirlpoolTick = bytemuck::from_bytes(tick_data);
                Ok(*tick)
            }
            TickArrayType::Dynamic => {
                // 🎯 Dynamic: 使用官方SDK的byte_offset算法
                let byte_offset = self.calculate_dynamic_byte_offset(offset as isize)?;
                self.get_dynamic_tick_at_byte_offset(byte_offset)
            }
        }
    }
    
    /// 🎯 计算Dynamic Tick Array的字节偏移 - 完全按照官方SDK实现
    /// 根据bitmap计算已初始化tick的数量，然后计算字节偏移
    fn calculate_dynamic_byte_offset(&self, tick_offset: isize) -> Result<usize> {
        if tick_offset < 0 {
            return Err(ErrorCode::InvalidTickIndex.into());
        }
        
        let data = self.account_info.try_borrow_data()?;
        // Dynamic: discriminator(8) + start_tick_index(4) + whirlpool(32) + tick_bitmap(16) + tick_data
        let bitmap_offset = 8 + 4 + 32; // 44 bytes
        if data.len() < bitmap_offset + 16 {
            return Err(ErrorCode::InvalidAccountData.into());
        }
        
        // 读取bitmap - 16字节 u128
        let bitmap_bytes: [u8; 16] = data[bitmap_offset..bitmap_offset + 16].try_into().unwrap();
        let tick_bitmap = u128::from_le_bytes(bitmap_bytes);
        
        // 🎯 官方SDK的byte_offset算法
        let mask = (1u128 << tick_offset) - 1;
        let initialized_ticks = (tick_bitmap & mask).count_ones() as usize;
        let uninitialized_ticks = tick_offset as usize - initialized_ticks;
        
        // 计算字节偏移：已初始化tick * 113字节 + 未初始化tick * 1字节
        let byte_offset = initialized_ticks * 113 + uninitialized_ticks * 1; // DynamicTick::INITIALIZED_LEN = 113, UNINITIALIZED_LEN = 1
        Ok(byte_offset)
    }
    
    /// 🎯 从字节偏移获取Dynamic tick数据 - 完全按照官方SDK实现
    fn get_dynamic_tick_at_byte_offset(&self, byte_offset: usize) -> Result<WhirlpoolTick> {
        let data = self.account_info.try_borrow_data()?;
        let tick_data_start = 8 + 4 + 32 + 16; // discriminator + start_tick_index + whirlpool + bitmap
        let actual_offset = tick_data_start + byte_offset;
        
        // 🎯 关键修复：按照官方SDK，总是尝试读取INITIALIZED_LEN长度
        if data.len() < actual_offset + DynamicTick::INITIALIZED_LEN {
            return Err(ErrorCode::InvalidDynTick.into());
        }
        
        // 🎯 按照官方SDK：使用DynamicTick::deserialize方法
        let mut tick_data = &data[actual_offset..actual_offset + DynamicTick::INITIALIZED_LEN];
        
        // 模拟DynamicTick::deserialize的行为
        let variant = tick_data[0];
        match variant {
            0 => {
                // Uninitialized variant
                Ok(WhirlpoolTick::default())
            }
            1 => {
                // Initialized variant - 解析DynamicTickData
                let liquidity_net = i128::from_le_bytes(tick_data[1..17].try_into().unwrap());
                let liquidity_gross = u128::from_le_bytes(tick_data[17..33].try_into().unwrap());
                let fee_growth_outside_a = u128::from_le_bytes(tick_data[33..49].try_into().unwrap());
                let fee_growth_outside_b = u128::from_le_bytes(tick_data[49..65].try_into().unwrap());
                
                let mut reward_growths_outside = [0u128; NUM_REWARDS];
                for i in 0..NUM_REWARDS {
                    let start = 65 + i * 16;
                    reward_growths_outside[i] = u128::from_le_bytes(tick_data[start..start + 16].try_into().unwrap());
                }
                
                Ok(WhirlpoolTick {
                    initialized: 1,
                    liquidity_net,
                    liquidity_gross,
                    fee_growth_outside_a,
                    fee_growth_outside_b,
                    reward_growths_outside,
                })
            }
            _ => {
                // 无效的variant
                Err(ErrorCode::InvalidDynTick.into())
            }
        }
    }
    
  
    /// 根据tick_index和tick_spacing获取tick
    pub fn get_tick(&self, tick_index: i32, tick_spacing: u16) -> Result<WhirlpoolTick> {
        let tick_offset = self.get_tick_offset(tick_index, tick_spacing)?;
        if tick_offset < 0 || tick_offset >= TICK_ARRAY_SIZE as isize {
            return Err(ErrorCode::InvalidTickIndex.into());
        }
        self.get_tick_by_offset(tick_offset as usize)
    }
    
    /// 计算tick在数组中的偏移量 - 完全按照官方SDK的get_offset实现
    pub fn get_tick_offset(&self, tick_index: i32, tick_spacing: u16) -> Result<isize> {
        if tick_spacing == 0 {
            return Err(ErrorCode::InvalidTickSpacing.into());
        }
        
        let start_tick_index = self.start_tick_index();
        // 完全按照官方SDK的get_offset实现
        let lhs = tick_index - start_tick_index;
        let rhs = tick_spacing as i32;
        let d = lhs / rhs;
        let r = lhs % rhs;
        let offset = if r < 0 { d - 1 } else { d };
        
        Ok(offset as isize)
    }
    
    /// 获取下一个初始化的 tick - 支持两种格式，与官方SDK逻辑一致
    pub fn get_next_initialized_tick_index(
        &self,
        tick_index: i32,
        tick_spacing: u16,
        a_to_b: bool,
    ) -> Option<i32> {
        match self.array_type {
            TickArrayType::Fixed => self.get_next_initialized_tick_fixed(tick_index, tick_spacing, a_to_b),
            TickArrayType::Dynamic => self.get_next_initialized_tick_dynamic(tick_index, tick_spacing, a_to_b),
        }
    }
    
    /// Fixed格式的下一个初始化tick搜索
    fn get_next_initialized_tick_fixed(&self, tick_index: i32, tick_spacing: u16, a_to_b: bool) -> Option<i32> {
        let start_tick_index = self.start_tick_index();
        let mut search_index = tick_index;
        
        if a_to_b {
            // 向左搜索（价格下降）
            while search_index >= start_tick_index {
                if let Ok(tick) = self.get_tick(search_index, tick_spacing) {
                    if tick.is_initialized() && search_index < tick_index {
                        return Some(search_index);
                    }
                }
                search_index -= tick_spacing as i32;
            }
        } else {
            // 向右搜索（价格上升）
            search_index += tick_spacing as i32;
            let max_tick = start_tick_index + TICK_ARRAY_SIZE as i32 * tick_spacing as i32;
            while search_index < max_tick {
                if let Ok(tick) = self.get_tick(search_index, tick_spacing) {
                    if tick.is_initialized() {
                        return Some(search_index);
                    }
                }
                search_index += tick_spacing as i32;
            }
        }
        
        None
    }
    
    /// Dynamic格式的下一个初始化tick搜索 - 使用bitmap优化，完全按照官方SDK实现
    fn get_next_initialized_tick_dynamic(&self, tick_index: i32, tick_spacing: u16, a_to_b: bool) -> Option<i32> {
        let start_tick_index = self.start_tick_index();
        
        // 检查搜索范围 - 完全按照官方SDK的in_search_range实现
        let mut lower = start_tick_index;
        let mut upper = start_tick_index + TICK_ARRAY_SIZE as i32 * tick_spacing as i32;
        if !a_to_b {
            // For b_to_a, shift the range by -tick_spacing
            lower -= tick_spacing as i32;
            upper -= tick_spacing as i32;
        }
        
        if tick_index < lower || tick_index >= upper {
            return None;
        }
        
        // 计算当前偏移
        let mut curr_offset = match self.get_tick_offset(tick_index, tick_spacing) {
            Ok(offset) => offset as i32,
            Err(_) => return None,
        };
        
        // For a_to_b searches, the search moves to the left. The next possible init-tick can be the 1st tick in the current offset
        // For b_to_a searches, the search moves to the right. The next possible init-tick cannot be within the current offset
        if !a_to_b {
            curr_offset += 1;
        }
        
        let bitmap = self.get_tick_bitmap().unwrap();
        while (0..TICK_ARRAY_SIZE as i32).contains(&curr_offset) {
            let initialized = (bitmap & (1u128 << curr_offset)) != 0;
            if initialized {
                return Some((curr_offset * tick_spacing as i32) + start_tick_index);
            }
            
            curr_offset = if a_to_b {
                curr_offset - 1
            } else {
                curr_offset + 1
            };
        }
        
        None
    }
}

/// Whirlpool 参数 - 包含 tick arrays (零拷贝版本)
pub struct WhirlpoolParams<'a> {
    pub tick_arrays: Vec<WhirlpoolTickArrayRef<'a>>,
    pub sqrt_price_limit: u128,
    pub oracle_info: Box<Option<AdaptiveFeeInfo>>,
}


// 常量定义 - 只保留Whirlpool特有的常量
const TICK_ARRAY_SIZE: usize = 88; // Whirlpool tick array size
const NUM_REWARDS: usize = 3;

/// 官方Tick结构 - 完全按照官方SDK定义 (113字节)
#[derive(Debug, Clone, Copy, Default, Pod, Zeroable)]
#[repr(C, packed)]
pub struct WhirlpoolTick {
    pub initialized: u8,                            // 1 byte (0 = false, 1 = true)
    pub liquidity_net: i128,                        // 16 bytes  
    pub liquidity_gross: u128,                      // 16 bytes
    pub fee_growth_outside_a: u128,                 // 16 bytes
    pub fee_growth_outside_b: u128,                 // 16 bytes
    pub reward_growths_outside: [u128; NUM_REWARDS], // 48 bytes (16 * 3)
}

impl WhirlpoolTick {
    /// 检查tick是否已初始化
    pub fn is_initialized(&self) -> bool {
        self.initialized != 0
    }
}

/// 官方FixedTickArray结构 - 完全按照官方SDK定义
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
#[repr(C, packed)]
pub struct WhirlpoolFixedTickArray {
    pub start_tick_index: i32,                      // 4 bytes
    pub ticks: [WhirlpoolTick; TICK_ARRAY_SIZE],     // 88 * 113 = 9944 bytes
    pub whirlpool: Pubkey,                          // 32 bytes
}

impl WhirlpoolTick {
    pub const LEN: usize = 113;
}

// 自适应费率相关常量
pub const FEE_RATE_HARD_LIMIT: u32 = 100_000; // 10%
pub const VOLATILITY_ACCUMULATOR_SCALE_FACTOR: u32 = 1000;
pub const ADAPTIVE_FEE_CONTROL_FACTOR_DENOMINATOR: u32 = 1_000_000;
pub const NO_EXPLICIT_SQRT_PRICE_LIMIT: u128 = 0u128;
pub const MAX_TICK_INDEX: i32 = 443636;
pub const MIN_TICK_INDEX: i32 = -443636;

/// 自适应费率常量
#[derive(Debug, Clone, Copy)]
pub struct AdaptiveFeeConstants {
    pub filter_period: u16,
    pub decay_period: u16,
    pub reduction_factor: u16,
    pub adaptive_fee_control_factor: u32,
    pub max_volatility_accumulator: u32,
    pub tick_group_size: u16,
    pub major_swap_threshold_ticks: u16,
}

/// 自适应费率变量
#[derive(Debug, Clone, Copy)]
pub struct AdaptiveFeeVariables {
    pub last_reference_update_timestamp: u64,
    pub last_major_swap_timestamp: u64,
    pub volatility_reference: u32,
    pub tick_group_index_reference: i32,
    pub volatility_accumulator: u32,
}

impl AdaptiveFeeVariables {
    pub fn update_reference(
        &mut self,
        tick_group_index: i32,
        timestamp: u64,
        constants: &AdaptiveFeeConstants,
    ) -> Result<()> {
        let time_delta = timestamp.saturating_sub(self.last_reference_update_timestamp);
        
        if time_delta >= constants.decay_period as u64 {
            // 重置为当前状态
            self.volatility_reference = self.volatility_accumulator;
            self.tick_group_index_reference = tick_group_index;
            self.last_reference_update_timestamp = timestamp;
        }
        
        Ok(())
    }
    
    pub fn update_volatility_accumulator(
        &mut self,
        tick_group_index: i32,
        constants: &AdaptiveFeeConstants,
    ) -> Result<()> {
        let tick_group_delta = (tick_group_index - self.tick_group_index_reference).abs();
        let volatility_delta = (tick_group_delta as u32).saturating_mul(VOLATILITY_ACCUMULATOR_SCALE_FACTOR);
        
        self.volatility_accumulator = min(
            self.volatility_reference.saturating_add(volatility_delta),
            constants.max_volatility_accumulator,
        );
        
        Ok(())
    }
    
    pub fn update_major_swap_timestamp(
        &mut self,
        pre_sqrt_price: u128,
        post_sqrt_price: u128,
        timestamp: u64,
        constants: &AdaptiveFeeConstants,
    ) -> Result<()> {
        // 计算价格变化对应的 tick 数量
        let pre_tick = tick_index_from_sqrt_price(pre_sqrt_price);
        let post_tick = tick_index_from_sqrt_price(post_sqrt_price);
        let tick_delta = (post_tick - pre_tick).abs();
        
        if tick_delta >= constants.major_swap_threshold_ticks as i32 {
            self.last_major_swap_timestamp = timestamp;
        }
        
        Ok(())
    }
}

/// 自适应费率信息
#[derive(Debug, Clone, Copy)]
pub struct AdaptiveFeeInfo {
    pub constants: AdaptiveFeeConstants,
    pub variables: AdaptiveFeeVariables,
}

// 数学函数 - 完全按照官方 Whirlpool SDK 实现
pub fn tick_index_from_sqrt_price(sqrt_price: u128) -> i32 {
    // 完全按照官方 SDK 的复杂对数计算
    const BIT_PRECISION: u32 = 14;
    const LOG_B_2_X32: i128 = 59543866431248i128;
    const LOG_B_P_ERR_MARGIN_LOWER_X64: i128 = 184467440737095516i128; // 0.01
    const LOG_B_P_ERR_MARGIN_UPPER_X64: i128 = 15793534762490258745i128; // 2^-precision / log_2_b + 0.01
    
    // 确定 log_b(sqrt_ratio)。首先计算整数部分 (msb)
    let msb: u32 = 128 - sqrt_price.leading_zeros() - 1;
    let log2p_integer_x32 = (msb as i128 - 64) << 32;

    // 获取小数值 (r/2^msb)，msb 总是 > 128
    // 我们从第 63 位开始迭代 (Q64.64 中的 0.5)
    let mut bit: i128 = 0x8000_0000_0000_0000i128;
    let mut precision = 0;
    let mut log2p_fraction_x64 = 0;

    // 小数部分的 Log2 迭代逼近
    // 遍历 Q64.64 数字中的每个 2^(j) 位，其中 j < 64
    // 如果 r^2 Q2.126 大于 2，则将当前位值附加到分数结果
    let mut r = if msb >= 64 {
        sqrt_price >> (msb - 63)
    } else {
        sqrt_price << (63 - msb)
    };

    while bit > 0 && precision < BIT_PRECISION {
        r *= r;
        let is_r_more_than_two = r >> 127_u32;
        r >>= 63 + is_r_more_than_two;
        log2p_fraction_x64 += bit * is_r_more_than_two as i128;
        bit >>= 1;
        precision += 1;
    }

    let log2p_fraction_x32 = log2p_fraction_x64 >> 32;
    let log2p_x32 = log2p_integer_x32 + log2p_fraction_x32;

    // 从基数 2 转换为基数 b
    let logbp_x64 = log2p_x32 * LOG_B_2_X32;

    // 导出 tick_low 和 high 估计值。调整可能低估 2^precision_bits/log_2(b) + 0.01 错误边距的可能性。
    let tick_low: i32 = ((logbp_x64 - LOG_B_P_ERR_MARGIN_LOWER_X64) >> 64)
        .try_into()
        .unwrap_or(MIN_TICK_INDEX);
    let tick_high: i32 = ((logbp_x64 + LOG_B_P_ERR_MARGIN_UPPER_X64) >> 64)
        .try_into()
        .unwrap_or(MAX_TICK_INDEX);

    if tick_low == tick_high {
        tick_low
    } else {
        // 如果我们对 tick_high 的估计返回比输入更低的 sqrt_price
        // 那么实际的 tick_high 必须高于 tick_high。
        // 否则，实际值在 tick_low 和 tick_high 之间，因此返回底值
        // (tick_low)
        let actual_tick_high_sqrt_price: u128 = sqrt_price_from_tick_index(tick_high);
        if actual_tick_high_sqrt_price <= sqrt_price {
            tick_high
        } else {
            tick_low
        }
    }
}

pub fn sqrt_price_from_tick_index(tick: i32) -> u128 {
    if tick >= 0 {
        get_sqrt_price_positive_tick(tick)
    } else {
        get_sqrt_price_negative_tick(tick)
    }
}

// 辅助函数：向下整数除法
pub fn floor_division(dividend: i32, divisor: i32) -> i32 {
    if divisor > 0 {
        if dividend >= 0 {
            dividend / divisor
        } else {
            (dividend - divisor + 1) / divisor
        }
    } else {
        panic!("Divisor must be positive");
    }
}

// 辅助函数：向上整数除法
pub fn ceil_division_u32(dividend: u32, divisor: u32) -> i32 {
    ((dividend + divisor - 1) / divisor) as i32
}

/// 费率管理器 - 支持静态和自适应费率
#[derive(Debug)]
pub enum FeeRateManager {
    Adaptive {
        a_to_b: bool,
        tick_group_index: i32,
        static_fee_rate: u16,
        adaptive_fee_constants: AdaptiveFeeConstants,
        adaptive_fee_variables: AdaptiveFeeVariables,
        core_tick_group_range_lower_bound: Option<(i32, u128)>,
        core_tick_group_range_upper_bound: Option<(i32, u128)>,
    },
    Static {
        static_fee_rate: u16,
    },
}

impl FeeRateManager {
    pub fn new(
        a_to_b: bool,
        current_tick_index: i32,
        timestamp: u64,
        static_fee_rate: u16,
        adaptive_fee_info: &Option<AdaptiveFeeInfo>,
    ) -> Result<Self> {
        match adaptive_fee_info {
            None => Ok(Self::Static { static_fee_rate }),
            Some(adaptive_fee_info) => {
                let tick_group_index = floor_division(
                    current_tick_index,
                    adaptive_fee_info.constants.tick_group_size as i32,
                );
                let adaptive_fee_constants = adaptive_fee_info.constants;
                let mut adaptive_fee_variables = adaptive_fee_info.variables;

                // 更新参考值
                adaptive_fee_variables.update_reference(
                    tick_group_index,
                    timestamp,
                    &adaptive_fee_constants,
                )?;

                // 计算核心 tick group 范围
                let max_volatility_accumulator_tick_group_index_delta = ceil_division_u32(
                    adaptive_fee_constants.max_volatility_accumulator
                        - adaptive_fee_variables.volatility_reference,
                    VOLATILITY_ACCUMULATOR_SCALE_FACTOR,
                );

                let core_tick_group_range_lower_index = adaptive_fee_variables
                    .tick_group_index_reference
                    - max_volatility_accumulator_tick_group_index_delta;
                let core_tick_group_range_upper_index = adaptive_fee_variables
                    .tick_group_index_reference
                    + max_volatility_accumulator_tick_group_index_delta;

                let core_tick_group_range_lower_bound_tick_index = core_tick_group_range_lower_index
                    * adaptive_fee_constants.tick_group_size as i32;
                let core_tick_group_range_upper_bound_tick_index = core_tick_group_range_upper_index
                    * adaptive_fee_constants.tick_group_size as i32
                    + adaptive_fee_constants.tick_group_size as i32;

                let core_tick_group_range_lower_bound = if core_tick_group_range_lower_bound_tick_index > -443636 {
                    Some((
                        core_tick_group_range_lower_index,
                        sqrt_price_from_tick_index(core_tick_group_range_lower_bound_tick_index),
                    ))
                } else {
                    None
                };

                let core_tick_group_range_upper_bound = if core_tick_group_range_upper_bound_tick_index < 443636 {
                    Some((
                        core_tick_group_range_upper_index,
                        sqrt_price_from_tick_index(core_tick_group_range_upper_bound_tick_index),
                    ))
                } else {
                    None
                };

                Ok(Self::Adaptive {
                    a_to_b,
                    tick_group_index,
                    static_fee_rate,
                    adaptive_fee_constants,
                    adaptive_fee_variables,
                    core_tick_group_range_lower_bound,
                    core_tick_group_range_upper_bound,
                })
            }
        }
    }

    pub fn update_volatility_accumulator(&mut self) -> Result<()> {
        match self {
            Self::Static { .. } => Ok(()),
            Self::Adaptive {
                tick_group_index,
                adaptive_fee_constants,
                adaptive_fee_variables,
                ..
            } => adaptive_fee_variables
                .update_volatility_accumulator(*tick_group_index, adaptive_fee_constants),
        }
    }

    pub fn advance_tick_group(&mut self) {
        match self {
            Self::Static { .. } => {
                // 静态费率不需要操作
            }
            Self::Adaptive {
                a_to_b,
                tick_group_index,
                ..
            } => {
                *tick_group_index += if *a_to_b { -1 } else { 1 };
            }
        }
    }

    pub fn advance_tick_group_after_skip(
        &mut self,
        sqrt_price: u128,
        next_tick_sqrt_price: u128,
        next_tick_index: i32,
    ) -> Result<()> {
        match self {
            Self::Static { .. } => {
                // 静态费率管理器不使用 skip 功能
                Ok(())
            }
            Self::Adaptive {
                a_to_b,
                tick_group_index,
                adaptive_fee_constants,
                adaptive_fee_variables,
                ..
            } => {
                let (tick_index, is_on_tick_group_boundary) = if sqrt_price == next_tick_sqrt_price {
                    let is_on_tick_group_boundary =
                        next_tick_index % adaptive_fee_constants.tick_group_size as i32 == 0;
                    (next_tick_index, is_on_tick_group_boundary)
                } else {
                    let tick_index = tick_index_from_sqrt_price(sqrt_price);
                    let is_on_tick_group_boundary =
                        tick_index % adaptive_fee_constants.tick_group_size as i32 == 0
                            && sqrt_price == sqrt_price_from_tick_index(tick_index);
                    (tick_index, is_on_tick_group_boundary)
                };

                let last_traversed_tick_group_index = if is_on_tick_group_boundary && !*a_to_b {
                    tick_index / adaptive_fee_constants.tick_group_size as i32 - 1
                } else {
                    floor_division(tick_index, adaptive_fee_constants.tick_group_size as i32)
                };

                if (*a_to_b && last_traversed_tick_group_index < *tick_group_index)
                    || (!*a_to_b && last_traversed_tick_group_index > *tick_group_index)
                {
                    *tick_group_index = last_traversed_tick_group_index;
                    // 🎯 关键差异：需要更新波动性累加器！
                    adaptive_fee_variables
                        .update_volatility_accumulator(*tick_group_index, adaptive_fee_constants)?;
                }

                *tick_group_index += if *a_to_b { -1 } else { 1 };

                Ok(())
            }
        }
    }

    pub fn get_total_fee_rate(&self) -> u32 {
        match self {
            Self::Static { static_fee_rate } => *static_fee_rate as u32,
            Self::Adaptive {
                static_fee_rate,
                adaptive_fee_constants,
                adaptive_fee_variables,
                ..
            } => {
                let adaptive_fee_rate =
                    Self::compute_adaptive_fee_rate(adaptive_fee_constants, adaptive_fee_variables);
                let total_fee_rate = *static_fee_rate as u32 + adaptive_fee_rate;

                if total_fee_rate > FEE_RATE_HARD_LIMIT {
                    FEE_RATE_HARD_LIMIT
                } else {
                    total_fee_rate
                }
            }
        }
    }

    pub fn get_bounded_sqrt_price_target(
        &self,
        sqrt_price: u128,
        curr_liquidity: u128,
    ) -> (u128, bool) {
        match self {
            Self::Static { .. } => (sqrt_price, false),
            Self::Adaptive {
                a_to_b,
                tick_group_index,
                adaptive_fee_constants,
                core_tick_group_range_lower_bound,
                core_tick_group_range_upper_bound,
                ..
            } => {
                // 如果自适应费率控制因子为0，跳过计算
                if adaptive_fee_constants.adaptive_fee_control_factor == 0 {
                    return (sqrt_price, true);
                }

                // 如果流动性为0，跳过计算
                if curr_liquidity == 0 {
                    return (sqrt_price, true);
                }

                // 检查是否在核心范围下界之外
                if let Some((lower_tick_group_index, lower_tick_group_bound_sqrt_price)) =
                    core_tick_group_range_lower_bound
                {
                    if *tick_group_index < *lower_tick_group_index {
                        if *a_to_b {
                            return (sqrt_price, true);
                        } else {
                            return (sqrt_price.min(*lower_tick_group_bound_sqrt_price), true);
                        }
                    }
                }

                // 检查是否在核心范围上界之外
                if let Some((upper_tick_group_index, upper_tick_group_bound_sqrt_price)) =
                    core_tick_group_range_upper_bound
                {
                    if *tick_group_index > *upper_tick_group_index {
                        if *a_to_b {
                            return (sqrt_price.max(*upper_tick_group_bound_sqrt_price), true);
                        } else {
                            return (sqrt_price, true);
                        }
                    }
                }

                let boundary_tick_index = if *a_to_b {
                    *tick_group_index * adaptive_fee_constants.tick_group_size as i32
                } else {
                    *tick_group_index * adaptive_fee_constants.tick_group_size as i32
                        + adaptive_fee_constants.tick_group_size as i32
                };

                let boundary_sqrt_price = sqrt_price_from_tick_index(boundary_tick_index);

                let bounded_sqrt_price = if *a_to_b {
                    sqrt_price.max(boundary_sqrt_price)
                } else {
                    sqrt_price.min(boundary_sqrt_price)
                };

                (bounded_sqrt_price, false)
            }
        }
    }

    fn compute_adaptive_fee_rate(
        constants: &AdaptiveFeeConstants,
        variables: &AdaptiveFeeVariables,
    ) -> u32 {
        // 完全按照官方SDK计算自适应费率
        let crossed = variables.volatility_accumulator * constants.tick_group_size as u32;
        let squared = crossed as u64 * crossed as u64;
        
        let fee_rate = (constants.adaptive_fee_control_factor as u128 * squared as u128)
            / (ADAPTIVE_FEE_CONTROL_FACTOR_DENOMINATOR as u128 
               * VOLATILITY_ACCUMULATOR_SCALE_FACTOR as u128 
               * VOLATILITY_ACCUMULATOR_SCALE_FACTOR as u128);
        
        if fee_rate > FEE_RATE_HARD_LIMIT as u128 {
            FEE_RATE_HARD_LIMIT
        } else {
            fee_rate as u32
        }
    }
}


/// 解析 Whirlpool 池数据 - 与其他 DEX 风格统一
pub fn parse_whirlpool_pool_data(
    pool_index: &usize,
    wsol_mint: Pubkey,
    token_mint: Pubkey,
    accounts: &[AccountInfo],
    pool_state: &mut WhirlpoolPoolState,
) -> Result<()> {
    let pool_account = &accounts[*pool_index];
    
    require!(
        *pool_account.owner == whirlpool_program_id::ID,
        ErrorCode::InvalidPoolOwner
    );
    
    let pool_data = pool_account.data.borrow();
    

    // 解析 Whirlpool 账户数据 (跳过 discriminator 8字节)
    // whirlpoolsConfig: 8-40 (32字节)
    // whirlpoolBump: 40-41 (1字节) 
    // tickSpacing: 41-43 (2字节)
    let tick_spacing = u16::from_le_bytes(pool_data[41..43].try_into().unwrap());
    
    // feeTierIndexSeed: 43-45 (2字节)
    // feeRate: 45-47 (2字节)  
    let fee_rate = u16::from_le_bytes(pool_data[45..47].try_into().unwrap());
    
    // protocolFeeRate: 47-49 (2字节)
    // liquidity: 49-65 (16字节)
    let liquidity = u128::from_le_bytes(pool_data[49..65].try_into().unwrap());
    
    // sqrtPrice: 65-81 (16字节)
    let sqrt_price = u128::from_le_bytes(pool_data[65..81].try_into().unwrap());
    
    // tickCurrentIndex: 81-85 (4字节)
    let tick_current_index = i32::from_le_bytes(pool_data[81..85].try_into().unwrap());
    
    // protocolFeeOwedA: 85-93 (8字节)
    // protocolFeeOwedB: 93-101 (8字节)
    // tokenMintA: 101-133 (32字节)
    let token_mint_a = Pubkey::try_from(&pool_data[101..133]).unwrap();
    
    // tokenVaultA: 133-165 (32字节)
    // feeGrowthGlobalA: 165-181 (16字节)
    // tokenMintB: 181-213 (32字节)
    let token_mint_b = Pubkey::try_from(&pool_data[181..213]).unwrap();
    
    // tokenVaultB: 213-245 (32字节)
    // feeGrowthGlobalB: 245-261 (16字节)
    // rewardLastUpdatedTimestamp: 261-269 (8字节)
    // rewardInfos: 269-653 (384字节 = 3 * 128字节)

    // 验证池子包含SOL和指定token的配对
    let (_, other_mint) = if token_mint_a == wsol_mint {
        (token_mint_a, token_mint_b)
    } else if token_mint_b == wsol_mint {
        (token_mint_b, token_mint_a)
    } else {
        return Err(ErrorCode::InvalidTokenPair.into());
    };

    require!(other_mint == token_mint, ErrorCode::TokenMismatch);

    // 填充池状态
    pool_state.token_mint_a = token_mint_a;
    pool_state.token_mint_b = token_mint_b;
    pool_state.sqrt_price = sqrt_price;
    pool_state.tick_spacing = tick_spacing;
    pool_state.tick_current_index = tick_current_index;
    pool_state.liquidity = liquidity;
    pool_state.trade_fee_rate = fee_rate as u64;

    // msg!("sqrt_price: {}", sqrt_price);
    // msg!("tick_current_index: {}", tick_current_index);
    // msg!("liquidity: {}", liquidity);
    // msg!("trade_fee_rate: {}", fee_rate);
    // msg!("token_mint_a: {}", token_mint_a);
    // msg!("token_mint_b: {}", token_mint_b);
    // msg!("tick_spacing: {}", tick_spacing);
    Ok(())
}



/// 解析 Whirlpool Tick Array 数据 - 支持Fixed和Dynamic两种格式
fn parse_whirlpool_tick_array<'a>(tick_array_info: &'a AccountInfo<'a>) -> Result<Option<WhirlpoolTickArrayRef<'a>>> {
    if *tick_array_info.owner != whirlpool_program_id::ID {
        msg!("Invalid whirlpool pool owner: {:?}", tick_array_info.key());
        return Ok(None);
    }

    let data = tick_array_info.try_borrow_data()?;
    
    // 检查最小长度：8 bytes discriminator
    if data.len() < 8 {
        msg!("Invalid whirlpool tick array (too small): {:?}", tick_array_info.key());
        return Ok(None);
    }

    // 🎯 关键改进：使用新的构造方式，自动检测格式
    match WhirlpoolTickArrayRef::new(tick_array_info) {
        Ok(tick_array_ref) => {
            // msg!("tick_array_address: {:?}, type: {:?}", tick_array_info.key(), tick_array_ref.array_type);
            // let start_tick_index = tick_array_ref.start_tick_index();
            // msg!("start_tick_index: {}", start_tick_index);
            Ok(Some(tick_array_ref))
        }
        Err(e) => {
            msg!("Failed to parse tick array {:?}: {:?}", tick_array_info.key(), e);
            Ok(None)
        }
    }
}


// /// 解析所有 Whirlpool Tick Arrays - 零拷贝版本
// pub fn parse_whirlpool_tick_arrays<'a>(
//     tick_array_infos: &'a [&'a AccountInfo<'a>],
// ) -> Result<Vec<WhirlpoolTickArrayRef<'a>>> {
//     let mut tick_arrays = Vec::with_capacity(3);
    
//     for &tick_array_info in tick_array_infos {
//         if let Some(tick_array_ref) = parse_whirlpool_tick_array(tick_array_info)? {
//             tick_arrays.push(tick_array_ref);
//         }
//     }
    
//     Ok(tick_arrays)
// }

/// 解析三个 Whirlpool Tick Arrays - 避免生命周期问题
pub fn parse_whirlpool_tick_arrays_three<'a>(
    tick_array_0: &'a AccountInfo<'a>,
    tick_array_1: &'a AccountInfo<'a>,
    tick_array_2: &'a AccountInfo<'a>,
) -> Result<Vec<WhirlpoolTickArrayRef<'a>>> {
    let mut tick_arrays = Vec::with_capacity(3);
    
    if let Some(tick_array_ref) = parse_whirlpool_tick_array(tick_array_0)? {
        tick_arrays.push(tick_array_ref);
    }
    if let Some(tick_array_ref) = parse_whirlpool_tick_array(tick_array_1)? {
        tick_arrays.push(tick_array_ref);
    }
    if let Some(tick_array_ref) = parse_whirlpool_tick_array(tick_array_2)? {
        tick_arrays.push(tick_array_ref);
    }
    
    Ok(tick_arrays)
}

/// 解析 Whirlpool Oracle 获取自适应费率信息 - 完全按照官方SDK
pub fn parse_whirlpool_oracle_adaptive_fee(oracle_info: &AccountInfo) -> Result<Option<AdaptiveFeeInfo>> {
    // 检查是否为未初始化的Oracle（系统程序拥有且数据为空）
    if oracle_info.owner == &anchor_lang::system_program::ID && oracle_info.data_is_empty() {
        return Ok(None); // Oracle未初始化，返回None但允许交易
    }
    
    if *oracle_info.owner != whirlpool_program_id::ID {
        return Ok(None); // 不是 Whirlpool Oracle
    }

    let data = oracle_info.try_borrow_data()?;
    const ORACLE_SIZE: usize = 8 + 32 + 8 + 32 + 56 + 128; // discriminator + whirlpool + timestamp + constants + variables + reserved
    
    if data.len() < ORACLE_SIZE {
        return Ok(None); // 数据不足，可能不是自适应费率池
    }

    // Oracle 结构：
    // discriminator: 8 bytes
    // whirlpool: 32 bytes (offset: 8)
    // trade_enable_timestamp: 8 bytes (offset: 40)
    // adaptive_fee_constants: 32 bytes (offset: 48)
    // adaptive_fee_variables: 56 bytes (offset: 80)
    
    // 解析 AdaptiveFeeConstants (32 bytes)
    let constants_offset = 48;
    let filter_period = u16::from_le_bytes(data[constants_offset..constants_offset + 2].try_into().unwrap());
    let decay_period = u16::from_le_bytes(data[constants_offset + 2..constants_offset + 4].try_into().unwrap());
    let reduction_factor = u16::from_le_bytes(data[constants_offset + 4..constants_offset + 6].try_into().unwrap());
    let adaptive_fee_control_factor = u32::from_le_bytes(data[constants_offset + 8..constants_offset + 12].try_into().unwrap());
    let max_volatility_accumulator = u32::from_le_bytes(data[constants_offset + 12..constants_offset + 16].try_into().unwrap());
    let tick_group_size = u16::from_le_bytes(data[constants_offset + 16..constants_offset + 18].try_into().unwrap());
    let major_swap_threshold_ticks = u16::from_le_bytes(data[constants_offset + 18..constants_offset + 20].try_into().unwrap());
    
    let constants = AdaptiveFeeConstants {
        filter_period,
        decay_period,
        reduction_factor,
        adaptive_fee_control_factor,
        max_volatility_accumulator,
        tick_group_size,
        major_swap_threshold_ticks,
    };
    
    // 解析 AdaptiveFeeVariables (56 bytes)
    let variables_offset = 80;
    let last_reference_update_timestamp = u64::from_le_bytes(data[variables_offset..variables_offset + 8].try_into().unwrap());
    let last_major_swap_timestamp = u64::from_le_bytes(data[variables_offset + 8..variables_offset + 16].try_into().unwrap());
    let volatility_reference = u32::from_le_bytes(data[variables_offset + 16..variables_offset + 20].try_into().unwrap());
    let tick_group_index_reference = i32::from_le_bytes(data[variables_offset + 20..variables_offset + 24].try_into().unwrap());
    let volatility_accumulator = u32::from_le_bytes(data[variables_offset + 24..variables_offset + 28].try_into().unwrap());
    
    let variables = AdaptiveFeeVariables {
        last_reference_update_timestamp,
        last_major_swap_timestamp,
        volatility_reference,
        tick_group_index_reference,
        volatility_accumulator,
    };

    msg!("constants: {:?}", constants);
    msg!("variables: {:?}", variables);
    
    // 注意：即使 adaptive_fee_control_factor == 0，也要返回AdaptiveFeeInfo
    // 因为官方SDK会在FeeRateManager中处理这种情况，而不是在Oracle解析时
    
    Ok(Some(AdaptiveFeeInfo {
        constants,
        variables,
    }))
}


/// 计算 Whirlpool 价格 - 
pub fn calculate_whirlpool_price(pool_state: &mut WhirlpoolPoolState, wsol_mint: Pubkey) -> Result<()> {
    if pool_state.liquidity == 0 {
        pool_state.price = 0;
        return Ok(());
    }

    let mut price_q64 = pool_state.sqrt_price;
    
    // sqrt_price^2 得到实际价格 (Q64格式) - 使用安全数学函数
    // price_q64 = price_q64.checked_mul(price_q64).unwrap().checked_shr(64).unwrap();
    price_q64 = safe_mul_shr_cast(price_q64, price_q64, 64, Rounding::Down);
   
    // 如果WSOL不是token0，需要取倒数
    if pool_state.token_mint_b != wsol_mint {
        // 使用安全的除法来计算倒数: (2^64)^2 / price_q64 = 2^128 / price_q64
        price_q64 = safe_mul_div_cast(ONE, ONE, price_q64, Rounding::Down);
        
    }
 
    pool_state.price = price_q64;
    Ok(())
}



// ======================== Tick Array 管理函数 ========================

// Tick Array 管理函数已移到 WhirlpoolTickArrayRef 实现中

// /// 查找包含指定 tick 的 tick array
// fn find_tick_array_for_tick(
//     tick_arrays: &[WhirlpoolTickArray],
//     tick_index: i32,
//     tick_spacing: u16,
// ) -> Option<usize> {
//     for (i, array) in tick_arrays.iter().enumerate() {
//         let max_tick = array.start_tick_index + TICK_ARRAY_SIZE * tick_spacing as i32;
//         if tick_index >= array.start_tick_index && tick_index < max_tick {
//             return Some(i);
//         }
//     }
//     None
// }

/// 获取下一个初始化的 tick - 从指定的数组索引开始搜索，跳过无效的tick arrays
fn get_next_initialized_tick<'a>(
    tick_arrays: &[WhirlpoolTickArrayRef<'a>],
    current_tick: i32,
    tick_spacing: u16,
    a_to_b: bool,
    start_array_index: usize,
) -> Result<(usize, i32)> {
    let mut array_index = start_array_index;
    let mut search_tick = current_tick;
    
    loop {
        // 确保数组索引有效
        if array_index >= tick_arrays.len() {
            return if a_to_b {
                Ok((tick_arrays.len().saturating_sub(1), MIN_TICK_INDEX))
            } else {
                Ok((tick_arrays.len().saturating_sub(1), MAX_TICK_INDEX))
            };
        }
        
        let array = &tick_arrays[array_index];
     
        
        // 在当前数组中搜索
        if let Some(next_tick) = array.get_next_initialized_tick_index(search_tick, tick_spacing, a_to_b) {
            return Ok((array_index, next_tick));
        }
        
        // 移动到下一个数组
        if a_to_b {
            if array_index == 0 {
                return Ok((0, MIN_TICK_INDEX));
            }
            array_index -= 1;
            // 设置搜索位置为新数组的末尾
            search_tick = array.start_tick_index() + (TICK_ARRAY_SIZE as i32 * tick_spacing as i32) - 1;
        } else {
            array_index += 1;
            if array_index >= tick_arrays.len() {
                return Ok((tick_arrays.len() - 1, MAX_TICK_INDEX));
            }
            // 设置搜索位置为新数组的开始
            search_tick = tick_arrays[array_index].start_tick_index();
        }
    }
}

// ======================== 完整的 Whirlpool Swap 实现 ========================

/// 获取下一个 sqrt 价格 - 完全按照官方 SDK
fn get_next_sqrt_prices(
    next_tick_index: i32,
    sqrt_price_limit: u128,
    a_to_b: bool,
) -> Result<(u128, u128)> {
    let next_tick_price = sqrt_price_from_tick_index(next_tick_index);
    let next_sqrt_price_limit = if a_to_b {
        sqrt_price_limit.max(next_tick_price)
    } else {
        sqrt_price_limit.min(next_tick_price)
    };
    Ok((next_tick_price, next_sqrt_price_limit))
}

/// 获取 tick 在 tick array 中的偏移 - 完全按照官方 SDK
fn get_tick_offset<'a>(
    array_index: usize,
    tick_index: i32,
    tick_spacing: u16,
    tick_arrays: &[WhirlpoolTickArrayRef<'a>],
) -> Result<isize> {
    if tick_spacing == 0 {
        return Err(ErrorCode::InvalidTickIndex.into());
    }
    
    let array = tick_arrays.get(array_index)
        .ok_or(ErrorCode::InvalidTickIndex)?;
    
    // 计算偏移 - 完全按照官方 get_offset 实现
    let lhs = tick_index - array.start_tick_index();
    let rhs = tick_spacing as i32;
    let d = lhs / rhs;
    let r = lhs % rhs;
    let offset = if r < 0 { d - 1 } else { d };
    
    Ok(offset as isize)
}

/// 完整的 Whirlpool swap 计算 - 包含 tick arrays 和跨 tick 逻辑
/// 只支持 amount_specified_is_input = true 的情况
fn whirlpool_swap_internal<'a>(
    pool_state: &WhirlpoolPoolState,
    tick_arrays: &[WhirlpoolTickArrayRef<'a>],
    amount: u64,
    sqrt_price_limit: u128,
    a_to_b: bool,
    adaptive_fee_info: &Option<AdaptiveFeeInfo>,
    timestamp: u64,
) -> Result<SwapUpdate> {

    // 设置价格限制
    let adjusted_sqrt_price_limit = if sqrt_price_limit == NO_EXPLICIT_SQRT_PRICE_LIMIT {
        if a_to_b {
            MIN_SQRT_PRICE_X64
        } else {
            MAX_SQRT_PRICE_X64
        }
    } else {
        sqrt_price_limit
    };
    
    // 验证价格限制方向
    if (a_to_b && adjusted_sqrt_price_limit >= pool_state.sqrt_price) ||
       (!a_to_b && adjusted_sqrt_price_limit <= pool_state.sqrt_price) {
        return Err(ErrorCode::InvalidSqrtPriceLimit.into());
    }
    
    let mut amount_remaining = amount;
    let mut amount_calculated = 0u64;
    let mut curr_sqrt_price = pool_state.sqrt_price;
    let mut curr_tick_index = pool_state.tick_current_index;
    let mut curr_liquidity = pool_state.liquidity;
    let mut curr_array_index = 0;
    let mut fee_sum = 0u64;

    // 初始化费率管理器
    let mut fee_rate_manager = FeeRateManager::new(
        a_to_b,
        pool_state.tick_current_index,
        timestamp,
        pool_state.trade_fee_rate as u16,
        adaptive_fee_info,
    )?;
    
    // 主 swap 循环
    while amount_remaining > 0 && adjusted_sqrt_price_limit != curr_sqrt_price {
        // 获取下一个初始化的 tick
        let (next_array_index, next_tick_index) = get_next_initialized_tick(
            tick_arrays,
            curr_tick_index,
            pool_state.tick_spacing,
            a_to_b,
            curr_array_index,
        )?;
        
        // 计算下一个 tick 的价格 - 完全按照官方SDK
        let (next_tick_sqrt_price, sqrt_price_target) = 
            get_next_sqrt_prices(next_tick_index, adjusted_sqrt_price_limit, a_to_b)?;
        
         
        loop {
            // 更新波动性累加器
            fee_rate_manager.update_volatility_accumulator()?;
            
            // 获取当前总费率（基础 + 自适应）
            let total_fee_rate = fee_rate_manager.get_total_fee_rate();
            
            // 获取有界目标价格
            let (bounded_sqrt_price_target, adaptive_fee_update_skipped) = 
                fee_rate_manager.get_bounded_sqrt_price_target(sqrt_price_target, curr_liquidity);
            
            // 执行单步 swap 计算
            let swap_computation = compute_swap_step(
                amount_remaining,
                total_fee_rate,
                curr_liquidity,
                curr_sqrt_price,
                bounded_sqrt_price_target,
                a_to_b,
            )?;
            
            // 更新数量 - 只支持 input 模式
            amount_remaining = amount_remaining
                    .checked_sub(swap_computation.amount_in)
                    .ok_or(ErrorCode::CalculateOverflow)?
                    .checked_sub(swap_computation.fee_amount)
                    .ok_or(ErrorCode::CalculateOverflow)?;
            amount_calculated = amount_calculated
                    .checked_add(swap_computation.amount_out)
                    .ok_or(ErrorCode::CalculateOverflow)?;
            
            fee_sum = fee_sum
                    .checked_add(swap_computation.fee_amount)
                    .ok_or(ErrorCode::CalculateOverflow)?;
            
            // 处理 tick 跨越 - 完全按照官方SDK
            if swap_computation.next_price == next_tick_sqrt_price {
                // 获取 tick 并更新流动性
                if let Some(array) = tick_arrays.get(next_array_index) {
                    if let Ok(tick) = array.get_tick(next_tick_index, pool_state.tick_spacing) {
                        if tick.is_initialized() {
                            let liquidity_net = if a_to_b {
                                -tick.liquidity_net
                            } else {
                                tick.liquidity_net
                            };
                            
                            curr_liquidity = if liquidity_net < 0 {
                                curr_liquidity.checked_sub((-liquidity_net) as u128)
                                    .ok_or(ErrorCode::LiquidityInsufficient)?
                            } else {
                                curr_liquidity.checked_add(liquidity_net as u128)
                                    .ok_or(ErrorCode::CalculateOverflow)?
                            };
                        }
                    }
                }
                
                // 计算 tick 在数组中的偏移 - 按照官方SDK
                let tick_offset = get_tick_offset(
                    next_array_index,
                    next_tick_index,
                    pool_state.tick_spacing,
                    tick_arrays,
                )?;
                
                // 更新数组索引 - 完全按照官方SDK逻辑
                // 移动到下一个 tick array 的条件：
                // - 价格向左移动且当前 tick 是 tick array 的开始
                // - 价格向右移动且当前 tick 是 tick array 的结束
                curr_array_index = if (a_to_b && tick_offset == 0)
                    || (!a_to_b && tick_offset == TICK_ARRAY_SIZE as isize - 1)
                {
                    next_array_index + 1
                } else {
                    next_array_index
                };
                
                // 更新当前 tick - 按照官方逻辑
                curr_tick_index = if a_to_b {
                    next_tick_index - 1
                } else {
                    next_tick_index
                };
            } else if swap_computation.next_price != curr_sqrt_price {
                // 价格改变了但没有跨越 tick
                curr_tick_index = tick_index_from_sqrt_price(swap_computation.next_price);
            }
            
            // 更新当前价格
            curr_sqrt_price = swap_computation.next_price;
            
            // 处理 tick group 更新 - 按照官方SDK
            if !adaptive_fee_update_skipped {
                fee_rate_manager.advance_tick_group();
            } else {
                fee_rate_manager.advance_tick_group_after_skip(
                    curr_sqrt_price,
                    next_tick_sqrt_price,
                    next_tick_index,
                )?;
            }
            
            // do while loop - 按照官方SDK
            if amount_remaining == 0 || curr_sqrt_price == sqrt_price_target {
                break;
            }
        }
    }
    
    // 计算最终结果
    let (amount_a, amount_b) = if a_to_b {
        (amount - amount_remaining, amount_calculated)
    } else {
        (amount_calculated, amount - amount_remaining)
    };
    
    Ok(SwapUpdate {
        amount_a,
        amount_b,
    })
}

/// Whirlpool Quote函数 - WSOL输入，Token输出 - 完整实现
pub fn whirlpool_quote_exact_input_wsol(
    pool_state: &WhirlpoolPoolState,
    wsol_mint: Pubkey,
    wsol_amount_in: u64,
    token_mint_info: &AccountInfo,
    whirlpool_params: &Option<WhirlpoolParams>,
) -> Result<u64> {
    if wsol_amount_in == 0 || pool_state.liquidity == 0 {
        return Ok(0);
    }
    

    let params = whirlpool_params.as_ref().ok_or(ErrorCode::InvalidClmmParams)?;
   
    if params.tick_arrays.is_empty() {
        return Ok(0);
    }


    // 确定交易方向
    let a_to_b = pool_state.token_mint_a == wsol_mint;
    
    // 获取当前时间戳
    let clock = Clock::get()?;
    let timestamp = clock.unix_timestamp as u64;
    
    // 执行完整的 swap 计算
    let swap_result = whirlpool_swap_internal(
        pool_state,
        &params.tick_arrays,
        wsol_amount_in,
        params.sqrt_price_limit,
        a_to_b,
        &params.oracle_info,
        timestamp,
    )?;
    
    let token_amount_out = if a_to_b { swap_result.amount_b } else { swap_result.amount_a };
    
    // 检查token 2022费用
    let transfer_fee = get_transfer_fee(token_mint_info, token_amount_out)?;
    let final_output = match transfer_fee {
        0 => token_amount_out,
        _ => token_amount_out.checked_sub(transfer_fee).unwrap_or(0),
    };

    Ok(final_output)
}

/// Whirlpool Quote函数 - Token输入，WSOL输出 - 完整实现
pub fn whirlpool_quote_exact_input_token(
    pool_state: &WhirlpoolPoolState,
    wsol_mint: Pubkey,
    token_amount_in: u64,
    token_mint_info: &AccountInfo,
    whirlpool_params: &Option<WhirlpoolParams>,
) -> Result<u64> {
    if token_amount_in == 0 || pool_state.liquidity == 0 {
        return Ok(0);
    }

    // 检查token 2022费用
    let transfer_fee = get_transfer_fee(token_mint_info, token_amount_in)?;
    let actual_token_amount_in = match transfer_fee {
        0 => token_amount_in,
        _ => token_amount_in.checked_sub(transfer_fee).unwrap_or(0),
    };

    if actual_token_amount_in == 0 {
        return Ok(0);
    }
    
    let params = whirlpool_params.as_ref().ok_or(ErrorCode::InvalidClmmParams)?;

    if params.tick_arrays.is_empty() {
        return Ok(0);
    }

    // 确定交易方向
    let a_to_b = pool_state.token_mint_b == wsol_mint;
    
    // 获取当前时间戳
    let clock = Clock::get()?;
    let timestamp = clock.unix_timestamp as u64;
    
  
    // 执行完整的 swap 计算
    let swap_result = whirlpool_swap_internal(
        pool_state,
        &params.tick_arrays,
        actual_token_amount_in,
        params.sqrt_price_limit,
        a_to_b,
        &params.oracle_info,
        timestamp,
    )?;
    
    let wsol_amount_out = if a_to_b { swap_result.amount_b } else { swap_result.amount_a };

    Ok(wsol_amount_out)
}




// ======================== Whirlpool Tick 价格转换函数 ========================

 

/// 正 tick 的 sqrt_price 计算 - 完全按照官方 Whirlpool SDK
fn get_sqrt_price_positive_tick(tick: i32) -> u128 {
    let mut ratio: u128 = if tick & 1 != 0 {
        79232123823359799118286999567
    } else {
        79228162514264337593543950336
    };

    if tick & 2 != 0 {
        ratio = mul_shift_96(ratio, 79236085330515764027303304731);
    }
    if tick & 4 != 0 {
        ratio = mul_shift_96(ratio, 79244008939048815603706035061);
    }
    if tick & 8 != 0 {
        ratio = mul_shift_96(ratio, 79259858533276714757314932305);
    }
    if tick & 16 != 0 {
        ratio = mul_shift_96(ratio, 79291567232598584799939703904);
    }
    if tick & 32 != 0 {
        ratio = mul_shift_96(ratio, 79355022692464371645785046466);
    }
    if tick & 64 != 0 {
        ratio = mul_shift_96(ratio, 79482085999252804386437311141);
    }
    if tick & 128 != 0 {
        ratio = mul_shift_96(ratio, 79736823300114093921829183326);
    }
    if tick & 256 != 0 {
        ratio = mul_shift_96(ratio, 80248749790819932309965073892);
    }
    if tick & 512 != 0 {
        ratio = mul_shift_96(ratio, 81282483887344747381513967011);
    }
    if tick & 1024 != 0 {
        ratio = mul_shift_96(ratio, 83390072131320151908154831281);
    }
    if tick & 2048 != 0 {
        ratio = mul_shift_96(ratio, 87770609709833776024991924138);
    }
    if tick & 4096 != 0 {
        ratio = mul_shift_96(ratio, 97234110755111693312479820773);
    }
    if tick & 8192 != 0 {
        ratio = mul_shift_96(ratio, 119332217159966728226237229890);
    }
    if tick & 16384 != 0 {
        ratio = mul_shift_96(ratio, 179736315981702064433883588727);
    }
    if tick & 32768 != 0 {
        ratio = mul_shift_96(ratio, 407748233172238350107850275304);
    }
    if tick & 65536 != 0 {
        ratio = mul_shift_96(ratio, 2098478828474011932436660412517);
    }
    if tick & 131072 != 0 {
        ratio = mul_shift_96(ratio, 55581415166113811149459800483533);
    }
    if tick & 262144 != 0 {
        ratio = mul_shift_96(ratio, 38992368544603139932233054999993551);
    }

    ratio >> 32
}

/// 负 tick 的 sqrt_price 计算 - 完全按照官方 Whirlpool SDK
fn get_sqrt_price_negative_tick(tick: i32) -> u128 {
    let abs_tick = tick.abs();

    let mut ratio: u128 = if abs_tick & 1 != 0 {
        18445821805675392311
    } else {
        18446744073709551616
    };

    if abs_tick & 2 != 0 {
        ratio = (ratio * 18444899583751176498) >> 64
    }
    if abs_tick & 4 != 0 {
        ratio = (ratio * 18443055278223354162) >> 64
    }
    if abs_tick & 8 != 0 {
        ratio = (ratio * 18439367220385604838) >> 64
    }
    if abs_tick & 16 != 0 {
        ratio = (ratio * 18431993317065449817) >> 64
    }
    if abs_tick & 32 != 0 {
        ratio = (ratio * 18417254355718160513) >> 64
    }
    if abs_tick & 64 != 0 {
        ratio = (ratio * 18387811781193591352) >> 64
    }
    if abs_tick & 128 != 0 {
        ratio = (ratio * 18329067761203520168) >> 64
    }
    if abs_tick & 256 != 0 {
        ratio = (ratio * 18212142134806087854) >> 64
    }
    if abs_tick & 512 != 0 {
        ratio = (ratio * 17980523815641551639) >> 64
    }
    if abs_tick & 1024 != 0 {
        ratio = (ratio * 17526086738831147013) >> 64
    }
    if abs_tick & 2048 != 0 {
        ratio = (ratio * 16651378430235024244) >> 64
    }
    if abs_tick & 4096 != 0 {
        ratio = (ratio * 15030750278693429944) >> 64
    }
    if abs_tick & 8192 != 0 {
        ratio = (ratio * 12247334978882834399) >> 64
    }
    if abs_tick & 16384 != 0 {
        ratio = (ratio * 8131365268884726200) >> 64
    }
    if abs_tick & 32768 != 0 {
        ratio = (ratio * 3584323654723342297) >> 64
    }
    if abs_tick & 65536 != 0 {
        ratio = (ratio * 696457651847595233) >> 64
    }
    if abs_tick & 131072 != 0 {
        ratio = (ratio * 26294789957452057) >> 64
    }
    if abs_tick & 262144 != 0 {
        ratio = (ratio * 37481735321082) >> 64
    }

    ratio
}

/// 乘法并右移96位 - 使用安全的数学运算避免溢出
fn mul_shift_96(n0: u128, n1: u128) -> u128 {
    // 使用ruint进行256位数学运算，避免溢出
    let product = ruint::aliases::U256::from(n0)
        .checked_mul(ruint::aliases::U256::from(n1))
        .unwrap_or(ruint::aliases::U256::MAX);
    
    // 右移96位
    let result: ruint::aliases::U256 = product >> 96;
    
    // 转换为u128，如果溢出则使用最大值
    if result > ruint::aliases::U256::from(u128::MAX) {
        u128::MAX
    } else {
        result.as_limbs()[0] as u128
    }
}

 

// ======================== Whirlpool 核心数学函数 ========================


/// 交换步骤计算结果
#[derive(Debug, Clone)]
struct SwapStepComputation {
    amount_in: u64,
    amount_out: u64,
    next_price: u128,
    fee_amount: u64,
}

/// Swap 更新结果
#[derive(Debug, Clone)]
struct SwapUpdate {
    amount_a: u64,
    amount_b: u64,
    // next_liquidity: u128,
    // next_tick_index: i32,
    // next_sqrt_price: u128,
}



/// 获取fixed token的数量变化
fn get_amount_fixed_delta(
    sqrt_price_current: u128,
    sqrt_price_target: u128,
    liquidity: u128,
    amount_specified_is_input: bool,
    a_to_b: bool,
) -> Result<u64> {
    if a_to_b == amount_specified_is_input {
        get_amount_delta_a(
            sqrt_price_current,
            sqrt_price_target,
            liquidity,
            amount_specified_is_input,
        ).map_err(|e| e.into())
    } else {
        get_amount_delta_b(
            sqrt_price_current,
            sqrt_price_target,
            liquidity,
            amount_specified_is_input,
        ).map_err(|e| e.into())
    }
}

/// 获取unfixed token的数量变化
fn get_amount_unfixed_delta(
    sqrt_price_current: u128,
    sqrt_price_target: u128,
    liquidity: u128,
    amount_specified_is_input: bool,
    a_to_b: bool,
) -> Result<u64> {
    if a_to_b == amount_specified_is_input {
        get_amount_delta_b(
            sqrt_price_current,
            sqrt_price_target,
            liquidity,
            !amount_specified_is_input,
        ).map_err(|e| e.into())
    } else {
        get_amount_delta_a(
            sqrt_price_current,
            sqrt_price_target,
            liquidity,
            !amount_specified_is_input,
        ).map_err(|e| e.into())
    }
}

/// 尝试获取 fixed delta - 返回 AmountDeltaU64 枚举
fn try_get_amount_fixed_delta(
    sqrt_price_current: u128,
    sqrt_price_target: u128,
    liquidity: u128,
    amount_specified_is_input: bool,
    a_to_b: bool,
) -> Result<AmountDeltaU64> {
    if a_to_b == amount_specified_is_input {
        try_get_amount_delta_a(
            sqrt_price_current,
            sqrt_price_target,
            liquidity,
            amount_specified_is_input,
        ).map_err(|e| e.into())
    } else {
        try_get_amount_delta_b(
            sqrt_price_current,
            sqrt_price_target,
            liquidity,
            amount_specified_is_input,
        ).map_err(|e| e.into())
    }
}



/// 核心swap步骤计算 - 完全按照Whirlpool SDK实现
/// 计算单个交换步骤 - 只支持 amount_specified_is_input = true
fn compute_swap_step(
    amount_remaining: u64,
    fee_rate: u32,
    liquidity: u128,
    sqrt_price_current: u128,
    sqrt_price_target: u128,
    a_to_b: bool,
) -> Result<SwapStepComputation> {
    if liquidity == 0 {
        return Ok(SwapStepComputation {
            amount_in: 0,
            amount_out: 0,
            next_price: sqrt_price_target,
            fee_amount: 0,
        });
    }

    // 🎯 关键修复：使用 try_get_amount_fixed_delta 处理可能的溢出
    let initial_amount_fixed_delta = try_get_amount_fixed_delta(
        sqrt_price_current,
        sqrt_price_target,
        liquidity,
        true, // amount_specified_is_input = true
        a_to_b,
    )?;

    // 🎯 关键修复：使用官方SDK的数学函数 - 扣除手续费后的可用数量
    // 只有在 amount_specified_is_input = true 时才计算扣除费用后的数量
    let amount_calc: u64 = if fee_rate == 0 {
        amount_remaining
    } else {
        ((amount_remaining as u128) * (FEE_RATE_MUL_VALUE - fee_rate as u128) / FEE_RATE_MUL_VALUE) as u64
    };

    // 🎯 关键修复：使用 lte 方法检查是否为 max swap
    let next_sqrt_price = if initial_amount_fixed_delta.lte(amount_calc) {
        sqrt_price_target
    } else {
        get_next_sqrt_price(
            sqrt_price_current,
            liquidity,
            amount_calc,
            true, // amount_specified_is_input = true
            a_to_b,
        )?
    };

    let is_max_swap = next_sqrt_price == sqrt_price_target;

    // 计算unfixed delta (amount_out)
    let amount_unfixed_delta = get_amount_unfixed_delta(
        sqrt_price_current,
        next_sqrt_price,
        liquidity,
        true, // amount_specified_is_input = true
        a_to_b,
    )?;

    // 🎯 关键修复：重新计算fixed delta - 如果不是max swap或初始计算溢出
    let amount_fixed_delta = if !is_max_swap || initial_amount_fixed_delta.exceeds_max() {
        // next_sqrt_price 是通过 get_next_sqrt_price 计算的，结果应该在u64范围内
        get_amount_fixed_delta(
            sqrt_price_current,
            next_sqrt_price,
            liquidity,
            true, // amount_specified_is_input = true
            a_to_b,
        )?
    } else {
        // 结果在u64范围内
        initial_amount_fixed_delta.value()
    };

    // amount_specified_is_input = true 时，固定输入，计算输出
    let amount_in = amount_fixed_delta;
    let amount_out = amount_unfixed_delta;

    // 🎯 关键修复：按照官方SDK计算手续费
    // 由于我们只支持 amount_specified_is_input = true，所以条件简化为 !is_max_swap
    let fee_amount = if !is_max_swap {
        // 非max swap时，手续费 = 剩余输入 - 实际输入
        msg!("amount_remaining: {}, amount_in: {}", amount_remaining, amount_in);
        amount_remaining - amount_in
    } else {
        // max swap时，根据输入量反推手续费 - 向上舍入
        if fee_rate == 0 {
            0
        } else {
            let fee_amount_128 = (amount_in as u128 * fee_rate as u128) / (FEE_RATE_MUL_VALUE - fee_rate as u128);
            let remainder = (amount_in as u128 * fee_rate as u128) % (FEE_RATE_MUL_VALUE - fee_rate as u128);
            if remainder > 0 {
                (fee_amount_128 + 1) as u64
            } else {
                fee_amount_128 as u64
            }
        }
    };

    Ok(SwapStepComputation {
        amount_in,
        amount_out,
        next_price: next_sqrt_price,
        fee_amount,
    })
}


 
/// 这个函数计算从当前价格到目标价格范围内能够输入或输出的最大SOL数量
pub fn calculate_whirlpool_amount_in_range(
    sqrt_price_current: u128,
    sqrt_price_target: u128,
    liquidity: u128,
    wsol_is_token_a: bool,
    round_up: bool,
) -> Result<Option<u64>> {
    if liquidity == 0 {
        return Ok(Some(0));
    }

    let result = if wsol_is_token_a {
        // SOL是token A，计算token A的数量变化
        get_amount_delta_a(
            sqrt_price_current,
            sqrt_price_target,
            liquidity,
            round_up, // input时向上舍入，output时向下舍入
        )
    } else {
        // SOL是token B，计算token B的数量变化
        get_amount_delta_b(
            sqrt_price_current,
            sqrt_price_target,
            liquidity,
            round_up, // input时向上舍入，output时向下舍入
        )
    };

    match result {
        Ok(amount) => Ok(Some(amount)),
        Err(e) => {
            return Err(e.into());
        }, 
    }
}