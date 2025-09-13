use crate::utils::errors::ErrorCode;
use crate::utils::u128x128_math::{safe_mul_div_cast, safe_mul_shr_cast, Rounding};
use crate::utils::u256x256_match::{div_rounding_up_u256, mul_div_ceil_u256, mul_div_floor_u256};
use crate::utils::u64x64_math::ONE;
use crate::utils::utils::get_transfer_fee;
use anchor_lang::prelude::*;
 

// 导入大数运算库
use ruint::aliases::{U1024, U256, U512};

pub mod clmm_program_id {
    use super::*;
    declare_id!("CAMMCzo5YL8w4VFF8KVHrK22GGUsp5VTaW7grrKgrWqK");
}

// 常量定义
const MIN_SQRT_PRICE_X64: u128 = 4295048016;
const MAX_SQRT_PRICE_X64: u128 = 79226673521066979257578248091;
const MIN_TICK: i32 = -443636;
const MAX_TICK: i32 = 443636;
const FEE_RATE_DENOMINATOR: u32 = 1_000_000;
const TICK_ARRAY_SIZE: i32 = 60;
const TICK_ARRAY_BITMAP_SIZE: i32 = 512;
const BIT_PRECISION: u32 = 16;
const EXTENSION_TICKARRAY_BITMAP_SIZE: usize = 14;

/// CLMM池状态数据结构
#[derive(Debug, Clone)]
pub struct ClmmPoolState {
    pub program_id_index: usize,

    pub pool_index: usize,
    pub amm_config_index: usize,
    pub observation_key_index: usize,
    pub token_vault_0_index: usize,
    pub token_vault_1_index: usize,
    pub tick_array_minus_1_index: usize,
    pub tick_array_0_index: usize,
    pub tick_array_1_index: usize,
    pub bitmap_extension_index: usize,

    pub token_mint_0: Pubkey,
    pub token_mint_1: Pubkey,

    pub sqrt_price_x64: u128,
    pub tick_current: i32,
    pub tick_spacing: u16,
    pub liquidity: u128,
    // pub fee_growth_global_0_x64: u128,
    // pub fee_growth_global_1_x64: u128,

    pub trade_fee_rate: u64,
    pub price: u128,
}
 
/// Bitmap Extension 数据结构 - 与Raydium SDK完全一致
#[derive(Debug, Clone)]
pub struct TickArrayBitmapExtension {
    pub pool_id: Pubkey,
    /// Packed initialized tick array state for start_tick_index is positive
    pub positive_tick_array_bitmap: [[u64; 8]; EXTENSION_TICKARRAY_BITMAP_SIZE],
    /// Packed initialized tick array state for start_tick_index is negative
    pub negative_tick_array_bitmap: [[u64; 8]; EXTENSION_TICKARRAY_BITMAP_SIZE],
}

impl TickArrayBitmapExtension {
    /// 解析bitmap extension账户数据
    pub fn parse_from_account_info(account_info: &AccountInfo) -> Result<Option<Box<Self>>> {
        if *account_info.owner != clmm_program_id::ID {
            return Ok(None);
        }

        let data = account_info.try_borrow_data()?;
        if data.len() < 8 + 32 + 64 * EXTENSION_TICKARRAY_BITMAP_SIZE * 2 {
            return Err(ErrorCode::InvalidAccountData.into());
        }

        // 跳过discriminator (8 bytes)
        let pool_id = Pubkey::try_from(&data[8..40]).map_err(|_| ErrorCode::InvalidAccountData)?;
        
        let mut positive_tick_array_bitmap = [[0u64; 8]; EXTENSION_TICKARRAY_BITMAP_SIZE];
        let mut negative_tick_array_bitmap = [[0u64; 8]; EXTENSION_TICKARRAY_BITMAP_SIZE];

        let mut offset = 40;
        // 解析positive bitmap
        for i in 0..EXTENSION_TICKARRAY_BITMAP_SIZE {
            for j in 0..8 {
                positive_tick_array_bitmap[i][j] = u64::from_le_bytes(
                    data[offset..offset + 8].try_into().map_err(|_| ErrorCode::InvalidAccountData)?
                );
                offset += 8;
            }
        }
        
        // 解析negative bitmap
        for i in 0..EXTENSION_TICKARRAY_BITMAP_SIZE {
            for j in 0..8 {
                negative_tick_array_bitmap[i][j] = u64::from_le_bytes(
                    data[offset..offset + 8].try_into().map_err(|_| ErrorCode::InvalidAccountData)?
                );
                offset += 8;
            }
        }

        Ok(Some(Box::new(Self {
            pool_id,
            positive_tick_array_bitmap,
            negative_tick_array_bitmap,
        })))
    }

    /// 检查tick array是否已初始化
    pub fn check_tick_array_is_initialized(
        &self,
        tick_array_start_index: i32,
        tick_spacing: u16,
    ) -> Result<(bool, i32)> {
        let (_, tickarray_bitmap) = self.get_bitmap(tick_array_start_index, tick_spacing)?;
        let tick_array_offset_in_bitmap = Self::tick_array_offset_in_bitmap(tick_array_start_index, tick_spacing);
        
        if U512::from_limbs(tickarray_bitmap).bit(tick_array_offset_in_bitmap as usize) {
            return Ok((true, tick_array_start_index));
        }
        Ok((false, tick_array_start_index))
    }

    /// 在一个bitmap中查找下一个初始化的tick array
    pub fn next_initialized_tick_array_from_one_bitmap_extension(
        &self,
        last_tick_array_start_index: i32,
        tick_spacing: u16,
        zero_for_one: bool,
    ) -> Result<(bool, i32)> {
        let multiplier = (tick_spacing as i32) * TICK_ARRAY_SIZE;
        let next_tick_array_start_index = if zero_for_one {
            last_tick_array_start_index - multiplier
        } else {
            last_tick_array_start_index + multiplier
        };
        
        let min_tick_array_start_index = get_array_start_index(MIN_TICK, tick_spacing);
        let max_tick_array_start_index = get_array_start_index(MAX_TICK, tick_spacing);

        if next_tick_array_start_index < min_tick_array_start_index
            || next_tick_array_start_index > max_tick_array_start_index
        {
            return Ok((false, next_tick_array_start_index));
        }

        let (_, tickarray_bitmap) = self.get_bitmap(next_tick_array_start_index, tick_spacing)?;
        
        Ok(Self::next_initialized_tick_array_in_bitmap(
            tickarray_bitmap,
            next_tick_array_start_index,
            tick_spacing,
            zero_for_one,
        ))
    }

    /// 在bitmap中查找下一个初始化的tick array - 静态方法
    pub fn next_initialized_tick_array_in_bitmap(
        tickarray_bitmap: [u64; 8],
        next_tick_array_start_index: i32,
        tick_spacing: u16,
        zero_for_one: bool,
    ) -> (bool, i32) {
        let (bitmap_min_tick_boundary, bitmap_max_tick_boundary) = 
            get_bitmap_tick_boundary(next_tick_array_start_index, tick_spacing);

        let tick_array_offset_in_bitmap = Self::tick_array_offset_in_bitmap(next_tick_array_start_index, tick_spacing);
        
        if zero_for_one {
            // tick从高到低查找
            let offset_bit_map = U512::from_limbs(tickarray_bitmap) << (TICK_ARRAY_BITMAP_SIZE - 1 - tick_array_offset_in_bitmap);
            
            let next_bit = if offset_bit_map.is_zero() {
                None
            } else {
                Some(offset_bit_map.leading_zeros() as u16)
            };

            if let Some(next_bit) = next_bit {
                let next_array_start_index = next_tick_array_start_index 
                    - (next_bit as i32) * (tick_spacing as i32) * TICK_ARRAY_SIZE;
                (true, next_array_start_index)
            } else {
                (false, bitmap_min_tick_boundary)
            }
        } else {
            // tick从低到高查找
            let offset_bit_map = U512::from_limbs(tickarray_bitmap) >> tick_array_offset_in_bitmap;
            
            let next_bit = if offset_bit_map.is_zero() {
                None
            } else {
                Some(offset_bit_map.trailing_zeros() as u16)
            };
            
            if let Some(next_bit) = next_bit {
                let next_array_start_index = next_tick_array_start_index
                    + (next_bit as i32) * (tick_spacing as i32) * TICK_ARRAY_SIZE;
                (true, next_array_start_index)
            } else {
                (false, bitmap_max_tick_boundary - (tick_spacing as i32) * TICK_ARRAY_SIZE)
            }
        }
    }

    /// 获取bitmap偏移量
    fn get_bitmap_offset(tick_index: i32, tick_spacing: u16) -> Result<usize> {
        Self::check_extension_boundary(tick_index, tick_spacing)?;
        let ticks_in_one_bitmap = max_tick_in_tickarray_bitmap(tick_spacing);
        let mut offset = tick_index.abs() / ticks_in_one_bitmap - 1;
        if tick_index < 0 && tick_index.abs() % ticks_in_one_bitmap == 0 {
            offset -= 1;
        }
        Ok(offset as usize)
    }

    /// 获取bitmap
    fn get_bitmap(&self, tick_index: i32, tick_spacing: u16) -> Result<(usize, [u64; 8])> {
        let offset = Self::get_bitmap_offset(tick_index, tick_spacing)?;
        if tick_index < 0 {
            Ok((offset, self.negative_tick_array_bitmap[offset]))
        } else {
            Ok((offset, self.positive_tick_array_bitmap[offset]))
        }
    }

    /// 检查是否在extension边界内
    pub fn check_extension_boundary(tick_index: i32, tick_spacing: u16) -> Result<()> {
        let positive_tick_boundary = max_tick_in_tickarray_bitmap(tick_spacing);
        
        require!(MAX_TICK > positive_tick_boundary, ErrorCode::InvalidTickIndex);
        require!(-positive_tick_boundary > MIN_TICK, ErrorCode::InvalidTickIndex);
        
        if tick_index >= -positive_tick_boundary && tick_index < positive_tick_boundary {
            return Err(ErrorCode::InvalidTickArrayBoundary.into());
        }
        Ok(())
    }

    /// 计算tick array在bitmap中的偏移量
    pub fn tick_array_offset_in_bitmap(tick_array_start_index: i32, tick_spacing: u16) -> i32 {
        let m = tick_array_start_index.abs() % max_tick_in_tickarray_bitmap(tick_spacing);
        let mut tick_array_offset_in_bitmap = m / ((tick_spacing as i32) * TICK_ARRAY_SIZE);
        if tick_array_start_index < 0 && m != 0 {
            tick_array_offset_in_bitmap = TICK_ARRAY_BITMAP_SIZE - tick_array_offset_in_bitmap;
        }
        tick_array_offset_in_bitmap
    }
}

#[derive(Debug, Clone)]
pub struct ClmmParams {
    pub tick_array_states: Box<Vec<TickArrayState>>,
    pub bitmap_extension: Option<Box<TickArrayBitmapExtension>>, // 提前解析的bitmap extension
    pub sqrt_price_limit_x64: u128,
    pub tick_array_bitmap: Box<[u64; 16]>,
}

/// 交换状态
#[derive(Default, Debug)]
struct SwapState {
    amount_specified_remaining: u64,
    amount_calculated: u64,
    sqrt_price_x64: u128,
    tick: i32,
    // fee_growth_global_x64: u128,
    // fee_amount: u64,
    liquidity: u128,
}

/// 交换步骤计算
#[derive(Default, Debug)]
struct StepComputations {
    sqrt_price_start_x64: u128,
    tick_next: i32,
    initialized: bool,
    sqrt_price_next_x64: u128,
    // amount_in: u64,
    // amount_out: u64,
    // fee_amount: u64,
}

/// 交换步骤结果
#[derive(Default, Debug)]
pub struct SwapStep {
    pub sqrt_price_next_x64: u128,
    pub amount_in: u64,
    pub amount_out: u64,
    pub fee_amount: u64,
}

/// Tick状态
#[derive(Default, Debug, Clone, Copy)]
pub struct TickState {
    pub tick: i32,
    pub liquidity_net: i128,
    pub liquidity_gross: u128,
    // pub fee_growth_outside_0_x64: u128,
    // pub fee_growth_outside_1_x64: u128,
    // pub reward_growths_outside_x64: [u128; 3],
    // pub initialized: bool,
}

impl TickState {
    pub fn new(tick: i32, liquidity_net: i128, liquidity_gross: u128) -> Self {
        Self {
            tick,
            liquidity_net,
            liquidity_gross,
        }
    }

    pub fn is_initialized(&self) -> bool {
        self.liquidity_gross > 0
    }

    // pub fn cross(
    //     &mut self,
    //     fee_growth_global_0_x64: u128,
    //     fee_growth_global_1_x64: u128,
    // ) -> i128 {
    //     self.fee_growth_outside_0_x64 = fee_growth_global_0_x64.wrapping_sub(self.fee_growth_outside_0_x64);
    //     self.fee_growth_outside_1_x64 = fee_growth_global_1_x64.wrapping_sub(self.fee_growth_outside_1_x64);

    //     // 更新 reward growth
    //     for i in 0..3 {
    //         self.reward_growths_outside_x64[i] = 0u128.wrapping_sub(self.reward_growths_outside_x64[i]);
    //     }

    //     self.liquidity_net
    // }
}

/// Tick Array 状态
#[derive(Debug, Clone)]
pub struct TickArrayState {
    pub start_tick_index: i32,
    pub ticks: Vec<TickState>,
    // pub pool_id: Pubkey,
}

impl TickArrayState {
    pub fn new(start_tick_index: i32, ticks: Vec<TickState>) -> Self {
        Self {
            start_tick_index,
            ticks,
            // pool_id,
        }
    }

    pub fn next_initialized_tick(
        &self,
        tick: i32,
        tick_spacing: u16,
        zero_for_one: bool,
    ) -> Result<Option<&TickState>> {
        // 检查当前tick是否在这个tick array范围内
        let current_tick_array_start_index = get_array_start_index(tick, tick_spacing);
        if current_tick_array_start_index != self.start_tick_index {
            return Ok(None);
        }

        // 关键修复：直接在数组索引上操作，而不是tick值！
        let mut offset_in_array = (tick - self.start_tick_index) / tick_spacing as i32;

        if zero_for_one {
            // 向左查找 - 直接操作数组索引，最多60次循环
            while offset_in_array >= 0 {
                if self.ticks[offset_in_array as usize].is_initialized() && 
                   self.ticks[offset_in_array as usize].tick < tick {
                    return Ok(Some(&self.ticks[offset_in_array as usize]));
                }
                offset_in_array -= 1;  // 关键：只减1，不是减tick_spacing
            }
        } else {
            // 向右查找 - 直接操作数组索引，最多60次循环
            offset_in_array += 1;
            while offset_in_array < TICK_ARRAY_SIZE {
                if self.ticks[offset_in_array as usize].is_initialized() {
                    return Ok(Some(&self.ticks[offset_in_array as usize]));
                }
                offset_in_array += 1;  // 关键：只加1，不是加tick_spacing
            }
        }

        Ok(None)
    }

    pub fn first_initialized_tick(&self, zero_for_one: bool) -> Result<&TickState> {
        if zero_for_one {
            // 从右到左查找
            for i in (0..TICK_ARRAY_SIZE).rev() {
                if self.ticks[i as usize].is_initialized() {
                    return Ok(&self.ticks[i as usize]);
                }
            }
        } else {
            // 从左到右查找
            for i in 0..TICK_ARRAY_SIZE {
                if self.ticks[i as usize].is_initialized() {
                    return Ok(&self.ticks[i as usize]);
                }
            }
        }

        Err(ErrorCode::LiquidityInsufficient.into())
    }
}

/// 解析CLMM池数据
pub fn parse_clmm_pool_data(
    pool_index: &usize,
    amm_config_index: &usize,
    wsol_mint: Pubkey,
    token_mint: Pubkey,
    accounts: &[AccountInfo],
    pool_state: &mut ClmmPoolState,
) -> Result<()> {
    let pool_account = &accounts[*pool_index];

    require!(
        *pool_account.owner == clmm_program_id::ID,
        ErrorCode::InvalidPoolOwner
    );
 
    // 解析基础池数据
    let (sqrt_price_x64, tick_current, tick_spacing, liquidity, token_mint_0, token_mint_1) = {
        let pool_data = pool_account.data.borrow();
  
        // 解析基础字段 (偏移量8开始，加上discriminator)
        let tick_spacing = u16::from_le_bytes(pool_data[235..237].try_into().unwrap());
        let liquidity = u128::from_le_bytes(pool_data[237..253].try_into().unwrap());
        let sqrt_price_x64 = u128::from_le_bytes(pool_data[253..269].try_into().unwrap());
        let tick_current = i32::from_le_bytes(pool_data[269..273].try_into().unwrap());
        // 解析mint地址 (偏移量73开始)
        let token_mint_0 = Pubkey::try_from(&pool_data[73..105]).unwrap();
        let token_mint_1 = Pubkey::try_from(&pool_data[105..137]).unwrap();
        (sqrt_price_x64, tick_current, tick_spacing, liquidity, token_mint_0, token_mint_1)
    };

    // 验证池子包含SOL和指定token的配对
    let (_, other_mint) = if token_mint_0 == wsol_mint {
        (token_mint_0, token_mint_1)
    } else if token_mint_1 == wsol_mint {
        (token_mint_1, token_mint_0)
    } else {
        return Err(ErrorCode::InvalidTokenPair.into());
    };


    require!(other_mint == token_mint, ErrorCode::TokenMismatch);


    // 解析amm_config fee
    let amm_config_account = &accounts[*amm_config_index];
    // let protocol_fee_rate = u32::from_le_bytes(amm_config_account.data.borrow()[35..39].try_into().unwrap());
    let trade_fee_rate = {
        let amm_config_data = amm_config_account.data.borrow();
        u32::from_le_bytes(amm_config_data[47..51].try_into().unwrap())
    };

    pool_state.trade_fee_rate = trade_fee_rate as u64;
    // tick_array_bitmap 现在在 ClmmParams 中管理
 

    // 填充池状态
    pool_state.token_mint_0 = token_mint_0;
    pool_state.token_mint_1 = token_mint_1;

    pool_state.sqrt_price_x64 = sqrt_price_x64;
    pool_state.tick_current = tick_current;
    pool_state.tick_spacing = tick_spacing;
    pool_state.liquidity = liquidity;
 

    Ok(())
}

/// 从池账户中提取 tick_array_bitmap
pub fn extract_tick_array_bitmap(pool_account: &AccountInfo) -> Result<[u64; 16]> {
    let pool_data = pool_account.data.borrow();
    if pool_data.len() < 1032 {
        return Err(ErrorCode::InvalidAccountData.into());
    }
    
    // 解析tick_array_bitmap (偏移量904开始，16个u64 = 128字节)
    let tick_array_bitmap: [u64; 16] = pool_data[904..1032]
        .chunks_exact(8)
        .map(|chunk| u64::from_le_bytes(chunk.try_into().unwrap()))
        .collect::<Vec<u64>>()
        .try_into()
        .unwrap();

    drop(pool_data);

    Ok(tick_array_bitmap)
}

/// 计算CLMM价格
pub fn calculate_clmm_price(pool_state: &mut ClmmPoolState, wsol_mint: Pubkey) -> Result<()> {
    let mut price_q64 = pool_state.sqrt_price_x64;

    // sqrt_price^2 得到实际价格 (Q64格式)
    // price_q64 = price_q64.checked_mul(price_q64).unwrap().checked_shr(64).unwrap();
    price_q64 = safe_mul_shr_cast(price_q64, price_q64, 64, Rounding::Down);

    // 如果WSOL不是token1，需要取倒数
    if pool_state.token_mint_1 != wsol_mint {
        // 使用安全的除法来计算倒数: (2^64)^2 / price_q64 = 2^128 / price_q64
        price_q64 = safe_mul_div_cast(ONE, ONE, price_q64, Rounding::Down);
    }
    // msg!("price_q64: {}", price_q64);
    pool_state.price = price_q64;

    Ok(())
}

/// 解析 Tick Array 状态 - 返回Option，未初始化时返回None
fn parse_tick_array_state(tick_array_info: &AccountInfo) -> Result<Option<TickArrayState>> {
    let data = tick_array_info.try_borrow_data()?;

    // 验证程序所有者 - 未初始化时跳过
    if *tick_array_info.owner != clmm_program_id::ID {
        msg!("{} not initialized", tick_array_info.key());
        return Ok(None);
    }

    // TickArrayState 布局：
    // discriminator: 8 bytes
    // pool_id: 32 bytes (offset: 8)
    // start_tick_index: 4 bytes (offset: 40)
    // ticks: [TickState; 60] (offset: 44)
    // initialized_tick_count: 1 byte (offset: 44 + 60*152 = 9164)
    // recent_epoch: 8 bytes (offset: 9165)
    // padding: 107 bytes (offset: 9173)

    // let pool_id = Pubkey::new_from_array(
    //     data[8..40]
    //         .try_into()
    //         .map_err(|_| ErrorCode::InvalidAccountData)?,
    // );

    let start_tick_index = i32::from_le_bytes(
        data[40..44]
            .try_into()
            .map_err(|_| ErrorCode::InvalidAccountData)?,
    );

    let mut ticks = Vec::with_capacity(TICK_ARRAY_SIZE as usize);

   
    const TICK_STATE_SIZE: usize = 168;

    for i in 0..TICK_ARRAY_SIZE {
        let offset = 44 + i as usize * TICK_STATE_SIZE;

        // TickState 布局：
        // tick: 4 bytes
        // liquidity_net: 16 bytes
        // liquidity_gross: 16 bytes
        // fee_growth_outside_0_x64: 16 bytes
        // fee_growth_outside_1_x64: 16 bytes
        // reward_growths_outside_x64: [u128; 3] = 48 bytes
        // padding: [u32; 13] = 52 bytes

        let tick = i32::from_le_bytes(
            data[offset..offset + 4]
                .try_into()
                .map_err(|_| ErrorCode::InvalidAccountData)?,
        );
        let liquidity_net = i128::from_le_bytes(
            data[offset + 4..offset + 20]
                .try_into()
                .map_err(|_| ErrorCode::InvalidAccountData)?,
        );
        let liquidity_gross = u128::from_le_bytes(
            data[offset + 20..offset + 36]
                .try_into()
                .map_err(|_| ErrorCode::InvalidAccountData)?,
        );
        // // 为了保持数据完整性，仍然解析这些字段，但它们对swap计算结果无影响
        // // cross函数只返回liquidity_net，fee_growth只用于LP收益计算
        // let fee_growth_outside_0_x64 = u128::from_le_bytes(
        //     data[offset+36..offset+52].try_into().map_err(|_| ErrorCode::InvalidAccountData)?
        // );
        // let fee_growth_outside_1_x64 = u128::from_le_bytes(
        //     data[offset+52..offset+68].try_into().map_err(|_| ErrorCode::InvalidAccountData)?
        // );

        // reward_growths_outside_x64 [u128; 3] - 对swap计算无关紧要
        // let mut reward_growths_outside_x64 = [0u128; 3];
        // for j in 0..3 {
        //     let reward_offset = offset + 68 + j * 16;
        //     reward_growths_outside_x64[j] = u128::from_le_bytes(
        //         data[reward_offset..reward_offset+16].try_into().map_err(|_| ErrorCode::InvalidAccountData)?
        //     );
        // }

        // let initialized = liquidity_gross > 0;
 
        ticks.push(TickState::new(tick, liquidity_net, liquidity_gross));
    }
    
   
    Ok(Some(TickArrayState::new(start_tick_index, ticks)))
}



/// CLMM Quote函数 - WSOL输入，Token输出
pub fn clmm_quote_exact_input_wsol(
    pool_state: &ClmmPoolState,
    wsol_mint: Pubkey,
    wsol_amount_in: u64,
    token_mint_info: &AccountInfo,
    clmm_params: &Option<ClmmParams>,
) -> Result<u64> {
    if wsol_amount_in == 0 || pool_state.liquidity == 0{
        return Ok(0);
    }
   

    let params = clmm_params.as_ref().ok_or(ErrorCode::InvalidClmmParams)?;

    // 确定交易方向
    let zero_for_one = if pool_state.token_mint_0 == wsol_mint {
        true // WSOL是token0，输出token1
    } else if pool_state.token_mint_1 == wsol_mint {
        false // WSOL是token1，输出token0
    } else {
        return Err(ErrorCode::InvalidTokenPair.into());
    };

    let sqrt_price_limit_x64 = if params.sqrt_price_limit_x64 == 0 {
        if zero_for_one {
            MIN_SQRT_PRICE_X64
        } else {
            MAX_SQRT_PRICE_X64
        }
    } else {
        params.sqrt_price_limit_x64
    };

    // 验证价格限制
    if zero_for_one {
        require!(
            sqrt_price_limit_x64 < pool_state.sqrt_price_x64
                && sqrt_price_limit_x64 >= MIN_SQRT_PRICE_X64,
            ErrorCode::InvalidSqrtPriceLimit
        );
    } else {
        require!(
            sqrt_price_limit_x64 > pool_state.sqrt_price_x64
                && sqrt_price_limit_x64 <= MAX_SQRT_PRICE_X64,
            ErrorCode::InvalidSqrtPriceLimit
        );
    }

   

    // 执行交换计算 - 直接使用原始数据，避免大量克隆造成栈溢出
    // let mut tick_array_states_clone = params.tick_array_states.clone();
    let (amount_0, amount_1) = clmm_swap_internal(
        pool_state,
        &params.tick_array_states,
        &params.tick_array_bitmap,
        &params.bitmap_extension, // 使用预解析的bitmap extension
        wsol_amount_in,
        sqrt_price_limit_x64,
        zero_for_one
    )?;

    let token_amount_out = if zero_for_one { amount_1 } else { amount_0 };

    // 检查token 2022费用
    let transfer_fee = get_transfer_fee(token_mint_info, token_amount_out)?;
    let final_output = match transfer_fee {
        0 => token_amount_out,
        _ => token_amount_out.checked_sub(transfer_fee).unwrap(),
    };

    Ok(final_output)
}

/// CLMM Quote函数 - Token输入，WSOL输出
pub fn clmm_quote_exact_input_token(
    pool_state: &ClmmPoolState,
    wsol_mint: Pubkey,
    token_amount_in: u64,
    token_mint_info: &AccountInfo,
    clmm_params: &Option<ClmmParams>,
) -> Result<u64> {
    
    if token_amount_in == 0 || pool_state.liquidity == 0{
        return Ok(0);
    }
     

    let params = clmm_params.as_ref().ok_or(ErrorCode::InvalidClmmParams)?;

    // 检查token 2022费用
    let transfer_fee = get_transfer_fee(token_mint_info, token_amount_in)?;
    let actual_token_amount_in = match transfer_fee {
        0 => token_amount_in,
        _ => token_amount_in.checked_sub(transfer_fee).unwrap(),
    };

    // 确定交易方向
    let zero_for_one = if pool_state.token_mint_0 == wsol_mint {
        false // Token是token1，输出token0(WSOL)
    } else if pool_state.token_mint_1 == wsol_mint {
        true // Token是token0，输出token1(WSOL)
    } else {
        return Err(ErrorCode::InvalidTokenPair.into());
    };

    let sqrt_price_limit_x64 = if params.sqrt_price_limit_x64 == 0 {
        if zero_for_one {
            MIN_SQRT_PRICE_X64
        } else {
            MAX_SQRT_PRICE_X64
        }
    } else {
        params.sqrt_price_limit_x64
    };

    // 验证价格限制
    if zero_for_one {
        require!(
            sqrt_price_limit_x64 < pool_state.sqrt_price_x64
                && sqrt_price_limit_x64 >= MIN_SQRT_PRICE_X64,
            ErrorCode::InvalidSqrtPriceLimit
        );
    } else {
        require!(
            sqrt_price_limit_x64 > pool_state.sqrt_price_x64
                && sqrt_price_limit_x64 <= MAX_SQRT_PRICE_X64,
            ErrorCode::InvalidSqrtPriceLimit
        );
    }
 

    // 执行交换计算 - 直接使用原始数据，避免大量克隆造成栈溢出
    // let mut tick_array_states_clone = params.tick_array_states.clone();
    let (amount_0, amount_1) = clmm_swap_internal(
        pool_state,
        &params.tick_array_states,
        &params.tick_array_bitmap,
        &params.bitmap_extension, // 使用预解析的bitmap extension
        actual_token_amount_in,
        sqrt_price_limit_x64, // sqrt_price_limit = 0 表示无价格限制
        zero_for_one,
        // true, // is_base_input
    )?;

    let wsol_amount_out = if zero_for_one { amount_1 } else { amount_0 };

    Ok(wsol_amount_out)
}


pub fn parse_tick_array_states(tick_arrays: &Vec<&AccountInfo>) -> Result<Vec<TickArrayState>> {
  
    // 创建 tick array 队列 - 全部解析
    let mut tick_array_states: Vec<TickArrayState> = Vec::new();
    for tick_array_info in tick_arrays {
        if let Some(tick_array_state) = parse_tick_array_state(tick_array_info)? {
            tick_array_states.push(tick_array_state);
        }
    }

    Ok(tick_array_states)
}


/// CLMM内部交换计算 - 完整准确版本
fn clmm_swap_internal(
    pool_state: &ClmmPoolState,
    tick_array_states: &Vec<TickArrayState>,
    tick_array_bitmap: &[u64; 16],
    bitmap_extension: &Option<Box<TickArrayBitmapExtension>>, // 使用预解析的bitmap extension
    amount_specified: u64,
    sqrt_price_limit_x64: u128,
    zero_for_one: bool,
) -> Result<(u64, u64)> {
    if amount_specified == 0 {
        return Err(ErrorCode::ZeroAmountSpecified.into());
    }
   
    // 初始化交换状态 - 使用Box减少栈使用
    let mut state = Box::new(SwapState {
        amount_specified_remaining: amount_specified,
        amount_calculated: 0,
        sqrt_price_x64: pool_state.sqrt_price_x64,
        tick: pool_state.tick_current,
        // fee_amount: 0,
        liquidity: pool_state.liquidity,
    });

 
    // 获取第一个有效的 tick array
    let (mut is_match_pool_current_tick_array, mut current_valid_tick_array_start_index) =
        get_first_initialized_tick_array(pool_state, tick_array_bitmap, bitmap_extension, zero_for_one)?;
    // let mut current_valid_tick_array_start_index = first_valid_tick_array_start_index;
 

    // 避免大量克隆，使用索引访问
    let mut current_tick_array_index = 0;
     
    // 找到第一个有效的 tick array - 使用索引而不是pop
    while current_tick_array_index < tick_array_states.len() {
        if tick_array_states[current_tick_array_index].start_tick_index == current_valid_tick_array_start_index {
            break;
        }
        current_tick_array_index += 1;
    }
    
    // 检查是否找到匹配的tick array
    if current_tick_array_index >= tick_array_states.len() {
        msg!("No match tick array1");
        return Ok((0, 0));
    }
    
    let mut tick_array_current = &tick_array_states[current_tick_array_index];
    
    // 主交换循环
    'outer: while state.amount_specified_remaining != 0 && state.sqrt_price_x64 != sqrt_price_limit_x64 {
        let mut step = Box::new(StepComputations::default());
        step.sqrt_price_start_x64 = state.sqrt_price_x64;

        // 查找下一个初始化的 tick - 按照Raydium方式
        let mut next_initialized_tick = if let Some(tick_state) = tick_array_current
            .next_initialized_tick(state.tick, pool_state.tick_spacing, zero_for_one)?
        {
            Box::new(*tick_state) 
        } else {
            if !is_match_pool_current_tick_array {
                is_match_pool_current_tick_array = true;
                Box::new(*tick_array_current.first_initialized_tick(zero_for_one)?) 
                //比如预先建池时），会先初始化一段 TickArray，但里面的 tick 全是空的，直到有人往这个区间加流动性时，才会看到 tick 被真正写入
            } else {
                Box::new(TickState::default())
            }
        };
        // 如果找不到初始化的 tick，切换到下一个 tick array
        if !next_initialized_tick.is_initialized() {
            let next_initialized_tickarray_index = next_initialized_tick_array_start_index(
                pool_state,
                tick_array_bitmap,
                bitmap_extension,
                current_valid_tick_array_start_index,
                zero_for_one,
            )?;
            if next_initialized_tickarray_index.is_none() {
                return Err(ErrorCode::LiquidityInsufficient.into());
            }

            while tick_array_current.start_tick_index != next_initialized_tickarray_index.unwrap() {
                current_tick_array_index += 1;
                if current_tick_array_index < tick_array_states.len() {
                    tick_array_current = &tick_array_states[current_tick_array_index];
                } else {
                    // 没有更多tick array了，按照当前输出返回
                    msg!("No more tick array");
                    break 'outer;
                }
            }
            current_valid_tick_array_start_index = next_initialized_tickarray_index.unwrap();

            let first_initialized_tick = tick_array_current.first_initialized_tick(zero_for_one)?;
            next_initialized_tick = Box::new(*first_initialized_tick);
        }

        step.tick_next = next_initialized_tick.tick;
        step.initialized = next_initialized_tick.is_initialized();
        // 边界检查
        if step.tick_next < MIN_TICK {
            step.tick_next = MIN_TICK;
        } else if step.tick_next > MAX_TICK {
            step.tick_next = MAX_TICK;
        }
        step.sqrt_price_next_x64 = get_sqrt_price_at_tick(step.tick_next)?;

        let target_price = if (zero_for_one && step.sqrt_price_next_x64 < sqrt_price_limit_x64)
            || (!zero_for_one && step.sqrt_price_next_x64 > sqrt_price_limit_x64)
        {
            sqrt_price_limit_x64 
            // msg!("sqrt_price_next_x64 out of range");
            // 这里需要是满足pool的刻度价格 我提供的是随机价格 所以这里直接break退出 
            // break;
        } else {
            step.sqrt_price_next_x64
        };
        // 计算交换步骤
        let swap_step = compute_swap_step(
            step.sqrt_price_start_x64,
            target_price,
            state.liquidity,
            state.amount_specified_remaining,
            pool_state.trade_fee_rate,
            true, // allways true
            zero_for_one,
        )?;

        state.sqrt_price_x64 = swap_step.sqrt_price_next_x64;
        // step.amount_in = swap_step.amount_in;
        // step.amount_out = swap_step.amount_out;
        // step.fee_amount = swap_step.fee_amount;

        // 更新状态
        state.amount_specified_remaining = state
            .amount_specified_remaining
            .checked_sub(swap_step.amount_in + swap_step.fee_amount)
            .unwrap();
        state.amount_calculated = state
            .amount_calculated
            .checked_add(swap_step.amount_out)
            .unwrap();

        // 更新费用增长
        // if state.liquidity > 0 {
            // let fee_growth_global_x64_delta =
            //     safe_mul_div_cast(step.fee_amount as u128, ONE, state.liquidity, Rounding::Up);
            // state.fee_growth_global_x64 = state
            //     .fee_growth_global_x64
            //     .checked_add(fee_growth_global_x64_delta)
            //     .unwrap();
            // state.fee_amount = state.fee_amount.checked_add(swap_step.fee_amount).unwrap();
        // }

        // 处理 tick 跨越
        if state.sqrt_price_x64 == step.sqrt_price_next_x64 {
            if step.initialized {
                // let mut liquidity_net = next_initialized_tick.cross(
                //     if zero_for_one {
                //         state.fee_growth_global_x64
                //     } else {
                //         pool_state.fee_growth_global_0_x64
                //     },
                //     if zero_for_one {
                //         pool_state.fee_growth_global_1_x64
                //     } else {
                //         state.fee_growth_global_x64
                //     },
                // );

                let mut liquidity_net = next_initialized_tick.liquidity_net;

                if zero_for_one {
                    liquidity_net = liquidity_net.wrapping_neg();
                }
                state.liquidity = add_delta(state.liquidity, liquidity_net)?;
            }

            state.tick = if zero_for_one {
                step.tick_next - 1
            } else {
                step.tick_next
            };
        } else if state.sqrt_price_x64 != step.sqrt_price_start_x64 {
            state.tick = get_tick_at_sqrt_price(state.sqrt_price_x64)?;
        }
    }

    // 返回结果
    let (amount_0, amount_1) = if zero_for_one {
        (
            amount_specified - state.amount_specified_remaining,
            state.amount_calculated,
        )
    } else {
        (
            state.amount_calculated,
            amount_specified - state.amount_specified_remaining,
        )
    };

    Ok((amount_0, amount_1))

}


// ======================== 数学计算库函数 ========================

/// 完整的交换步骤计算 - 与官方完全一致
fn compute_swap_step(
    sqrt_price_current_x64: u128,
    sqrt_price_target_x64: u128,
    liquidity: u128,
    amount_remaining: u64,
    fee_rate: u64,
    is_base_input: bool,
    zero_for_one: bool,
) -> Result<SwapStep> {
    let mut swap_step = SwapStep::default();

    // 扣除手续费后的实际数量
    let amount_remaining_less_fee = safe_mul_div_cast(
        amount_remaining as u128,
        (FEE_RATE_DENOMINATOR - fee_rate as u32) as u128,
        FEE_RATE_DENOMINATOR as u128,
        Rounding::Down,
    ) as u64;

    // 预先计算价格范围内的输入量
    let amount_in_range = calculate_amount_in_range(
        sqrt_price_current_x64,
        sqrt_price_target_x64,
        liquidity,
        zero_for_one,
        is_base_input,
    )?;

    if let Some(amount_in) = amount_in_range {
        swap_step.amount_in = amount_in;
    }

    // 计算下一个价格
    swap_step.sqrt_price_next_x64 =
        if amount_in_range.is_some() && amount_remaining_less_fee >= swap_step.amount_in {
            sqrt_price_target_x64
        } else {
            get_next_sqrt_price_from_input(
                sqrt_price_current_x64,
                liquidity,
                amount_remaining_less_fee,
                zero_for_one,
            )?
        };

    // 是否达到最大价格
    let max = sqrt_price_target_x64 == swap_step.sqrt_price_next_x64;

    // 计算输入/输出数量
    if zero_for_one {
        if !(max && is_base_input) {
            swap_step.amount_in = get_delta_amount_0_unsigned(
                swap_step.sqrt_price_next_x64,
                sqrt_price_current_x64,
                liquidity,
                true,
            )?;
        }
        if !(max && !is_base_input) {
            swap_step.amount_out = get_delta_amount_1_unsigned(
                swap_step.sqrt_price_next_x64,
                sqrt_price_current_x64,
                liquidity,
                false,
            )?;
        }
    } else {
        if !(max && is_base_input) {
            swap_step.amount_in = get_delta_amount_1_unsigned(
                sqrt_price_current_x64,
                swap_step.sqrt_price_next_x64,
                liquidity,
                true,
            )?;
        }
        if !(max && !is_base_input) {
            swap_step.amount_out = get_delta_amount_0_unsigned(
                sqrt_price_current_x64,
                swap_step.sqrt_price_next_x64,
                liquidity,
                false,
            )?;
        }
    }

    // 限制精确输出的输出量
    if !is_base_input && swap_step.amount_out > amount_remaining {
        swap_step.amount_out = amount_remaining;
    }

    // 计算手续费
    swap_step.fee_amount =
        if is_base_input && swap_step.sqrt_price_next_x64 != sqrt_price_target_x64 {
            // 没有达到目标价格，剩余输入作为手续费
            amount_remaining.checked_sub(swap_step.amount_in).unwrap()
        } else {
            // 按比例计算手续费
            safe_mul_div_cast(
                swap_step.amount_in as u128,
                fee_rate as u128,
                (FEE_RATE_DENOMINATOR - fee_rate as u32) as u128,
                Rounding::Up,
            ) as u64
        };

    Ok(swap_step)
}

/// 预计算价格范围内的数量
pub fn calculate_amount_in_range(
    sqrt_price_current_x64: u128,
    sqrt_price_target_x64: u128,
    liquidity: u128,
    zero_for_one: bool,
    is_base_input: bool,
) -> Result<Option<u64>> {
    if is_base_input {
        let result = if zero_for_one {
            get_delta_amount_0_unsigned(
                sqrt_price_target_x64,
                sqrt_price_current_x64,
                liquidity,
                true,
            )
        } else {
            get_delta_amount_1_unsigned(
                sqrt_price_current_x64,
                sqrt_price_target_x64,
                liquidity,
                true,
            )
        };

        match result {
            Ok(amount) => Ok(Some(amount)),
            Err(e) => {
                if e == ErrorCode::MaxTokenOverflow.into() {
                    Ok(None)
                } else {
                    Err(ErrorCode::SqrtPriceLimitOverflow.into())
                }
            }
        }
    } else {
        let result = if zero_for_one {
            get_delta_amount_1_unsigned(
                sqrt_price_target_x64,
                sqrt_price_current_x64,
                liquidity,
                false,
            )
        } else {
            get_delta_amount_0_unsigned(
                sqrt_price_current_x64,
                sqrt_price_target_x64,
                liquidity,
                false,
            )
        };

        match result {
            Ok(amount) => Ok(Some(amount)),
            Err(e) => {
                if e == ErrorCode::MaxTokenOverflow.into() {
                    Ok(None)
                } else {
                    Err(ErrorCode::SqrtPriceLimitOverflow.into())
                }
            }
        }
    }
}

// ======================== 辅助函数 ========================

/// 获取第一个初始化的 tick array - 完整实现
fn get_first_initialized_tick_array(
    pool_state: &ClmmPoolState,
    tick_array_bitmap: &[u64; 16],
    bitmap_extension: &Option<Box<TickArrayBitmapExtension>>, // 使用预解析的bitmap extension
    zero_for_one: bool,
) -> Result<(bool, i32)> {
    let current_tick_array_start_index = get_array_start_index(pool_state.tick_current, pool_state.tick_spacing);
    
    // 检查是否超出默认bitmap范围
    let (is_initialized, start_index) = if is_overflow_default_tickarray_bitmap(pool_state.tick_current, pool_state.tick_spacing) {
        // 使用预解析的extension bitmap
        if let Some(ext) = bitmap_extension {
            ext.check_tick_array_is_initialized(current_tick_array_start_index, pool_state.tick_spacing)?
        } else {
            return Err(ErrorCode::InvalidAccountData.into());
        }
    } else {
        // 使用默认bitmap
        check_current_tick_array_is_initialized(
            U1024::from_limbs(*tick_array_bitmap),
            pool_state.tick_current,
            pool_state.tick_spacing,
        )?
    };

    if is_initialized {
        return Ok((true, start_index));
    }

    // 如果当前tick array未初始化，查找下一个初始化的tick array
    let next_start_index = next_initialized_tick_array_start_index(
        pool_state,
        tick_array_bitmap,
        bitmap_extension,
        current_tick_array_start_index,
        zero_for_one,
    )?;
    
    require!(
        next_start_index.is_some(),
        ErrorCode::LiquidityInsufficient
    );
    
    Ok((false, next_start_index.unwrap()))
}

/// 查找下一个初始化的 tick array 起始索引 - 完整实现
fn next_initialized_tick_array_start_index(
    pool_state: &ClmmPoolState,
    tick_array_bitmap: &[u64; 16],
    bitmap_extension: &Option<Box<TickArrayBitmapExtension>>, // 使用预解析的bitmap extension
    mut last_tick_array_start_index: i32,
    zero_for_one: bool,
) -> Result<Option<i32>> {
    // 确保start_index是有效的tick array起始索引
    last_tick_array_start_index =
        get_array_start_index(last_tick_array_start_index, pool_state.tick_spacing);

    loop {
        // 首先在池子的默认bitmap中查找
        let (is_found, start_index) = next_initialized_tick_array_start_index_internal(
            U1024::from_limbs(*tick_array_bitmap),
            last_tick_array_start_index,
            pool_state.tick_spacing,
            zero_for_one,
        );

        if is_found {
            return Ok(Some(start_index));
        }
        
        last_tick_array_start_index = start_index;

        // 如果在默认bitmap中没找到，尝试预解析的extension bitmap
        if let Some(ext) = bitmap_extension {
            let (is_found, start_index) = ext.next_initialized_tick_array_from_one_bitmap_extension(
                last_tick_array_start_index,
                pool_state.tick_spacing,
                zero_for_one,
            )?;
            
            if is_found {
                return Ok(Some(start_index));
            }
            
            last_tick_array_start_index = start_index;
        }

        if last_tick_array_start_index < -443636 || last_tick_array_start_index > 443636
        {
            return Ok(None);
        }
    }
}

/// 从 tick 获取 sqrt_price - 完全按照 Raydium SDK 实现
/// 计算 1.0001^(tick/2) 作为 Q64.64 数字，代表两个资产比率的平方根
fn get_sqrt_price_at_tick(tick: i32) -> Result<u128> {
    let abs_tick = tick.abs() as u32;
    require!(abs_tick <= MAX_TICK as u32, ErrorCode::InvalidTickIndex);

    // 模拟SDK的U128结构，使用u128直接计算
    // i = 0
    let mut ratio = if abs_tick & 0x1 != 0 {
        0xfffcb933bd6fb800u128
    } else {
        // 2^64
        1u128 << 64
    };

    // i = 1 to 18 的位操作计算，完全按照SDK逻辑
    if abs_tick & 0x2 != 0 {
        ratio = mul_shr_64(ratio, 0xfff97272373d4000u128);
    }
    if abs_tick & 0x4 != 0 {
        ratio = mul_shr_64(ratio, 0xfff2e50f5f657000u128);
    }
    if abs_tick & 0x8 != 0 {
        ratio = mul_shr_64(ratio, 0xffe5caca7e10f000u128);
    }
    if abs_tick & 0x10 != 0 {
        ratio = mul_shr_64(ratio, 0xffcb9843d60f7000u128);
    }
    if abs_tick & 0x20 != 0 {
        ratio = mul_shr_64(ratio, 0xff973b41fa98e800u128);
    }
    if abs_tick & 0x40 != 0 {
        ratio = mul_shr_64(ratio, 0xff2ea16466c9b000u128);
    }
    if abs_tick & 0x80 != 0 {
        ratio = mul_shr_64(ratio, 0xfe5dee046a9a3800u128);
    }
    if abs_tick & 0x100 != 0 {
        ratio = mul_shr_64(ratio, 0xfcbe86c7900bb000u128);
    }
    if abs_tick & 0x200 != 0 {
        ratio = mul_shr_64(ratio, 0xf987a7253ac65800u128);
    }
    if abs_tick & 0x400 != 0 {
        ratio = mul_shr_64(ratio, 0xf3392b0822bb6000u128);
    }
    if abs_tick & 0x800 != 0 {
        ratio = mul_shr_64(ratio, 0xe7159475a2caf000u128);
    }
    if abs_tick & 0x1000 != 0 {
        ratio = mul_shr_64(ratio, 0xd097f3bdfd2f2000u128);
    }
    if abs_tick & 0x2000 != 0 {
        ratio = mul_shr_64(ratio, 0xa9f746462d9f8000u128);
    }
    if abs_tick & 0x4000 != 0 {
        ratio = mul_shr_64(ratio, 0x70d869a156f31c00u128);
    }
    if abs_tick & 0x8000 != 0 {
        ratio = mul_shr_64(ratio, 0x31be135f97ed3200u128);
    }
    if abs_tick & 0x10000 != 0 {
        ratio = mul_shr_64(ratio, 0x9aa508b5b85a500u128);
    }
    if abs_tick & 0x20000 != 0 {
        ratio = mul_shr_64(ratio, 0x5d6af8dedc582cu128);
    }
    if abs_tick & 0x40000 != 0 {
        ratio = mul_shr_64(ratio, 0x2216e584f5fau128);
    }

    // 除法获得 1.0001^(tick/2)，与SDK完全一致
    if tick > 0 {
        ratio = u128::MAX / ratio;
    }

    Ok(ratio)
}

/// 辅助函数：u128 乘法然后右移64位，模拟 (a * b) >> 64
#[inline]
fn mul_shr_64(a: u128, b: u128) -> u128 {
    // 使用 U256 进行中间计算以避免溢出
    let result = U256::from(a) * U256::from(b);
    let shifted: U256 = result >> 64;

    // 取低128位
    u256_to_u128(shifted)
}

/// 辅助函数：将U256转换为u128，只取低128位
#[inline]
fn u256_to_u128(value: U256) -> u128 {
    let limbs = value.as_limbs();
    ((limbs[1] as u128) << 64) | (limbs[0] as u128)
}

/// 根据 token0 数量计算下一个 sqrt_price (向上舍入)
/// 公式: √P' = √P * L / (L + Δx * √P) 或 √P' = L / (L/√P + Δx)
fn get_next_sqrt_price_from_amount_0_rounding_up(
    sqrt_price_x64: u128,
    liquidity: u128,
    amount: u64,
    add: bool,
) -> u128 {
    if amount == 0 {
        return sqrt_price_x64;
    }

    // numerator_1 = liquidity << 64 (Q64.64 格式)
    let numerator_1 = U256::from(liquidity) << 64;

    if add {
        // 尝试计算 amount * sqrt_price_x64，检查溢出
        if let Some(product) = U256::from(amount).checked_mul(U256::from(sqrt_price_x64)) {
            let denominator = numerator_1 + product;
            if denominator >= numerator_1 {
                // 使用标准公式: √P' = √P * L / (L + Δx * √P)
                let result =
                    mul_div_ceil_u256(numerator_1, U256::from(sqrt_price_x64), denominator)
                        .unwrap();
                return u256_to_u128(result);
            }
        }

        // 溢出情况，使用备选公式: √P' = L / (L/√P + Δx)
        let quotient = numerator_1 / U256::from(sqrt_price_x64);
        let denominator = quotient.checked_add(U256::from(amount)).unwrap();
        u256_to_u128(div_rounding_up_u256(numerator_1, denominator))
    } else {
        // subtract 情况
        let product = U256::from(amount)
            .checked_mul(U256::from(sqrt_price_x64))
            .unwrap();
        let denominator = numerator_1.checked_sub(product).unwrap();
        let result =
            mul_div_ceil_u256(numerator_1, U256::from(sqrt_price_x64), denominator).unwrap();
        u256_to_u128(result)
    }
}

/// 根据 token1 数量计算下一个 sqrt_price (向下舍入)
/// 公式: √P' = √P + Δy / L
fn get_next_sqrt_price_from_amount_1_rounding_down(
    sqrt_price_x64: u128,
    liquidity: u128,
    amount: u64,
    add: bool,
) -> u128 {
    if add {
        // √P' = √P + Δy / L
        let quotient = (U256::from(amount as u128) << 64) / U256::from(liquidity);
        sqrt_price_x64.checked_add(u256_to_u128(quotient)).unwrap()
    } else {
        // √P' = √P - Δy / L
        let quotient =
            div_rounding_up_u256(U256::from(amount as u128) << 64, U256::from(liquidity));
        sqrt_price_x64.checked_sub(u256_to_u128(quotient)).unwrap()
    }
}

/// 从 sqrt_price 获取 tick - 精确实现
/// 计算最大的tick值使得 get_sqrt_price_at_tick(tick) <= ratio
fn get_tick_at_sqrt_price(sqrt_price_x64: u128) -> Result<i32> {
    // 验证边界条件
    if sqrt_price_x64 < MIN_SQRT_PRICE_X64 || sqrt_price_x64 >= MAX_SQRT_PRICE_X64 {
        return Err(ErrorCode::InvalidSqrtPrice.into());
    }

    // 确定log_b(sqrt_ratio)。首先计算整数部分(msb)
    let msb: u32 = 128 - sqrt_price_x64.leading_zeros() - 1;
    let log2p_integer_x32 = (msb as i128 - 64) << 32;

    // 获取小数值 (r/2^msb), msb总是 > 64
    // 从位63开始迭代 (Q64.64中的0.5)
    let mut bit: i128 = 0x8000_0000_0000_0000i128;
    let mut precision = 0;
    let mut log2p_fraction_x64 = 0;

    // Log2 小数部分的迭代近似
    // 遍历Q64.64数字中j < 64的每个2^(j)位
    // 如果r^2 Q2.126大于2，则将当前位值附加到分数结果
    let mut r = if msb >= 64 {
        sqrt_price_x64 >> (msb - 63)
    } else {
        sqrt_price_x64 << (63 - msb)
    };

    while bit > 0 && precision < BIT_PRECISION {
        r *= r;
        let is_r_more_than_two = r >> 127 as u32;
        r >>= 63 + is_r_more_than_two;
        log2p_fraction_x64 += bit * is_r_more_than_two as i128;
        bit >>= 1;
        precision += 1;
    }

    let log2p_fraction_x32 = log2p_fraction_x64 >> 32;
    let log2p_x32 = log2p_integer_x32 + log2p_fraction_x32;

    // 14位细化给出2^-14 / log2 (√1.0001) = 0.8461 < 1的误差边界
    // 由于tick是十进制，小于1的误差是可以接受的

    // 基数变换规则：乘以 2^32 / log2 (√1.0001)
    let log_sqrt_10001_x64 = log2p_x32 * 59543866431248i128;

    // tick - 0.01
    let tick_low = ((log_sqrt_10001_x64 - 184467440737095516i128) >> 64) as i32;

    // tick + (2^-14 / log2(√1.0001)) + 0.01
    let tick_high = ((log_sqrt_10001_x64 + 15793534762490258745i128) >> 64) as i32;

    if tick_low == tick_high {
        Ok(tick_low)
    } else if get_sqrt_price_at_tick(tick_high)? <= sqrt_price_x64 {
        Ok(tick_high)
    } else {
        Ok(tick_low)
    }
}

/// 从输入计算下一个 sqrt_price
fn get_next_sqrt_price_from_input(
    sqrt_price_x64: u128,
    liquidity: u128,
    amount_in: u64,
    zero_for_one: bool,
) -> Result<u128> {
    require!(sqrt_price_x64 > 0, ErrorCode::InvalidSqrtPrice);
    require!(liquidity > 0, ErrorCode::LiquidityInsufficient);

    if zero_for_one {
        // token0 -> token1: 使用 get_next_sqrt_price_from_amount_0_rounding_up
        Ok(get_next_sqrt_price_from_amount_0_rounding_up(
            sqrt_price_x64,
            liquidity,
            amount_in,
            true,
        ))
    } else {
        // token1 -> token0: 使用 get_next_sqrt_price_from_amount_1_rounding_down
        Ok(get_next_sqrt_price_from_amount_1_rounding_down(
            sqrt_price_x64,
            liquidity,
            amount_in,
            true,
        ))
    }
}

 

/// 计算 delta amount 0 (unsigned) - 使用U256高精度计算
/// 公式: Δx = L * (√P_upper - √P_lower) / (√P_upper * √P_lower)
pub fn get_delta_amount_0_unsigned(
    sqrt_price_a_x64: u128,
    sqrt_price_b_x64: u128,
    liquidity: u128,
    round_up: bool,
) -> Result<u64> {
    let (sqrt_price_lower, sqrt_price_upper) = if sqrt_price_a_x64 > sqrt_price_b_x64 {
        (sqrt_price_b_x64, sqrt_price_a_x64)
    } else {
        (sqrt_price_a_x64, sqrt_price_b_x64)
    };

    if sqrt_price_lower == 0 {
        return Err(ErrorCode::InvalidSqrtPrice.into());
    }

    // 使用U256进行高精度计算，避免溢出
    // numerator_1 = liquidity << 64 (Q64.64格式)
    let numerator_1 = U256::from(liquidity) << 64;
    // numerator_2 = sqrt_price_upper - sqrt_price_lower
    let numerator_2 = U256::from(sqrt_price_upper - sqrt_price_lower);

    let result = if round_up {
        // 向上舍入: ceil((numerator_1 * numerator_2) / sqrt_price_upper) / sqrt_price_lower)
        let intermediate =
            mul_div_ceil_u256(numerator_1, numerator_2, U256::from(sqrt_price_upper))?;
        div_rounding_up_u256(intermediate, U256::from(sqrt_price_lower))
    } else {
        // 向下舍入: floor((numerator_1 * numerator_2) / sqrt_price_upper) / sqrt_price_lower)
        let intermediate =
            mul_div_floor_u256(numerator_1, numerator_2, U256::from(sqrt_price_upper))?;
        intermediate / U256::from(sqrt_price_lower)
    };

    if result > U256::from(u64::MAX) {
        return Err(ErrorCode::MaxTokenOverflow.into());
    }

    Ok(result.as_limbs()[0])
}

/// 计算 delta amount 1 (unsigned) - 使用U256高精度计算
/// 公式: Δy = L * (√P_upper - √P_lower) / Q64
pub fn get_delta_amount_1_unsigned(
    sqrt_price_a_x64: u128,
    sqrt_price_b_x64: u128,
    liquidity: u128,
    round_up: bool,
) -> Result<u64> {
    let (sqrt_price_lower, sqrt_price_upper) = if sqrt_price_a_x64 > sqrt_price_b_x64 {
        (sqrt_price_b_x64, sqrt_price_a_x64)
    } else {
        (sqrt_price_a_x64, sqrt_price_b_x64)
    };

    // 使用U256进行高精度计算
    let result = if round_up {
        // 向上舍入: ceil(liquidity * (sqrt_price_upper - sqrt_price_lower) / Q64)
        mul_div_ceil_u256(
            U256::from(liquidity),
            U256::from(sqrt_price_upper - sqrt_price_lower),
            U256::from(1u128 << 64), // Q64 = 2^64
        )?
    } else {
        // 向下舍入: floor(liquidity * (sqrt_price_upper - sqrt_price_lower) / Q64)
        mul_div_floor_u256(
            U256::from(liquidity),
            U256::from(sqrt_price_upper - sqrt_price_lower),
            U256::from(1u128 << 64), // Q64 = 2^64
        )?
    };

    if result > U256::from(u64::MAX) {
        return Err(ErrorCode::MaxTokenOverflow.into());
    }

    Ok(result.as_limbs()[0])
}

/// 添加流动性增量
fn add_delta(liquidity: u128, liquidity_delta: i128) -> Result<u128> {
    if liquidity_delta < 0 {
        let delta = (-liquidity_delta) as u128;
        liquidity
            .checked_sub(delta)
            .ok_or(ErrorCode::LiquidityInsufficient.into())
    } else {
        let delta = liquidity_delta as u128;
        liquidity
            .checked_add(delta)
            .ok_or(ErrorCode::CalculateOverflow.into())
    }
}

// ======================== Bitmap 辅助函数 ========================

/// 检查当前tick对应的tick array是否初始化
fn check_current_tick_array_is_initialized(
    bit_map: U1024,
    tick_current: i32,
    tick_spacing: u16,
) -> Result<(bool, i32)> {
    if tick_current < MIN_TICK || tick_current > MAX_TICK {
        return Err(ErrorCode::InvalidTickIndex.into());
    }

    let multiplier = (tick_spacing as i32) * TICK_ARRAY_SIZE;
    let mut compressed = tick_current / multiplier + 512;
    if tick_current < 0 && tick_current % multiplier != 0 {
        // 向负无穷舍入
        compressed -= 1;
    }

    let bit_pos = compressed.abs();
    // 设置当前位
    let mask = U1024::from(1u128) << bit_pos;
    let masked = bit_map & mask;
    // 检查当前位是否初始化
    let initialized = !masked.is_zero();

    let start_index = (compressed - 512) * multiplier;
    Ok((initialized, start_index))
}

/// 在bitmap中查找下一个初始化的tick array起始索引
fn next_initialized_tick_array_start_index_internal(
    bit_map: U1024,
    last_tick_array_start_index: i32,
    tick_spacing: u16,
    zero_for_one: bool,
) -> (bool, i32) {
    let tick_boundary = max_tick_in_tickarray_bitmap(tick_spacing);
    let multiplier = (tick_spacing as i32) * TICK_ARRAY_SIZE;

    let next_tick_array_start_index = if zero_for_one {
        last_tick_array_start_index - multiplier
    } else {
        last_tick_array_start_index + multiplier
    };

    if next_tick_array_start_index < -tick_boundary || next_tick_array_start_index >= tick_boundary
    {
        return (false, last_tick_array_start_index);
    }

    let mut compressed = next_tick_array_start_index / multiplier + 512;
    if next_tick_array_start_index < 0 && next_tick_array_start_index % multiplier != 0 {
        // 向负无穷舍入
        compressed -= 1;
    }

    // let bit_pos = compressed.abs();

    if zero_for_one {
        // tick从高到低
        // 从高位到低位查找
        let offset_bit_map = bit_map << (1024 - compressed.abs() - 1);
        let next_bit = most_significant_bit(offset_bit_map);
        if let Some(next_bit) = next_bit {
            let next_array_start_index = (compressed.abs() - next_bit as i32 - 512) * multiplier;
            (true, next_array_start_index)
        } else {
            // 找不到，到边界
            (false, -tick_boundary)
        }
    } else {
        // tick从低到高
        // 从低位到高位查找
        let offset_bit_map = bit_map >> compressed.abs();
        let next_bit = least_significant_bit(offset_bit_map);
        if let Some(next_bit) = next_bit {
            let next_array_start_index = (compressed.abs() + next_bit as i32 - 512) * multiplier;
            (true, next_array_start_index)
        } else {
            // 找不到，到边界
            (false, tick_boundary - multiplier)
        }
    }
}

/// 获取bitmap中最高有效位
fn most_significant_bit(x: U1024) -> Option<u16> {
    if x.is_zero() {
        None
    } else {
        Some(x.leading_zeros() as u16)
    }
}

/// 获取bitmap中最低有效位
fn least_significant_bit(x: U1024) -> Option<u16> {
    if x.is_zero() {
        None
    } else {
        Some(x.trailing_zeros() as u16)
    }
}

/// 计算bitmap中最大tick
fn max_tick_in_tickarray_bitmap(tick_spacing: u16) -> i32 {
    (tick_spacing as i32) * TICK_ARRAY_SIZE * TICK_ARRAY_BITMAP_SIZE
}

/// 获取tick array的起始索引
pub fn get_array_start_index(tick_index: i32, tick_spacing: u16) -> i32 {
    let ticks_in_array = TICK_ARRAY_SIZE * i32::from(tick_spacing);
    let mut start = tick_index / ticks_in_array;
    if tick_index < 0 && tick_index % ticks_in_array != 0 {
        start = start - 1
    }
    start * ticks_in_array
}

/// 获取bitmap边界 - 与SDK一致
fn get_bitmap_tick_boundary(tick_array_start_index: i32, tick_spacing: u16) -> (i32, i32) {
    let ticks_in_one_bitmap = max_tick_in_tickarray_bitmap(tick_spacing);
    
    if tick_array_start_index >= 0 {
        let bitmap_start = (tick_array_start_index / ticks_in_one_bitmap) * ticks_in_one_bitmap;
        (bitmap_start, bitmap_start + ticks_in_one_bitmap)
    } else {
        let bitmap_end = ((tick_array_start_index + 1) / ticks_in_one_bitmap) * ticks_in_one_bitmap;
        (bitmap_end - ticks_in_one_bitmap, bitmap_end)
    }
}

/// 检查当前tick是否超出默认bitmap范围
fn is_overflow_default_tickarray_bitmap(tick_index: i32, tick_spacing: u16) -> bool {
    let tick_array_start_index = get_array_start_index(tick_index, tick_spacing);
    let max_tick_boundary = max_tick_in_tickarray_bitmap(tick_spacing);
    
    tick_array_start_index >= max_tick_boundary || tick_array_start_index < -max_tick_boundary
}

//计算clmm最大sol数量变化
pub fn calculate_clmm_amount_in_range(
    sqrt_price_current: u128,
    sqrt_price_target: u128,
    liquidity: u128,
    wsol_is_token_0: bool,
    round_up: bool,
) -> Result<Option<u64>> {
    if liquidity == 0 {
        return Ok(Some(0));
    }

    let result = if wsol_is_token_0 {
        // SOL是token A，计算token A的数量变化
        get_delta_amount_0_unsigned(
            sqrt_price_current,
            sqrt_price_target,
            liquidity,
            round_up, // input时向上舍入，output时向下舍入
        )
    } else {
        // SOL是token B，计算token B的数量变化
        get_delta_amount_1_unsigned(
            sqrt_price_current,
            sqrt_price_target,
            liquidity,
            round_up, // input时向上舍入，output时向下舍入
        )
    };

    match result {
        Ok(amount) => Ok(Some(amount)),
        Err(e) => {
            if e == ErrorCode::MaxTokenOverflow.into() {
                Ok(None)  
            } else {
                Err(e)
            }
        }
    }
}