use ruint::aliases::{U256, U512};
use crate::utils::u64x64_math::{ONE, SCALE_OFFSET};
use crate::constant::BASE_BPS;
use anchor_lang::prelude::*;

#[derive(Clone, Debug, PartialEq)]
pub enum Rounding {
    Up,
    Down,
}

/// (x * y) / denominator
fn mul_div(x: u128, y: u128, denominator: u128, rounding: Rounding) -> Option<u128> {
    if denominator == 0 {
        return None;
    }

    let x = U256::from(x);
    let y = U256::from(y);
    let denominator = U256::from(denominator);

    let prod = x.checked_mul(y)?;

    match rounding {
        Rounding::Up => prod.div_ceil(denominator).try_into().ok(),
        Rounding::Down => {
            let (quotient, _) = prod.div_rem(denominator);
            quotient.try_into().ok()
        }
    }
}

/// (x * y) >> offset
#[inline]
fn mul_shr(x: u128, y: u128, offset: u8, rounding: Rounding) -> Option<u128> {
    let denominator = 1u128.checked_shl(offset.into())?;
    mul_div(x, y, denominator, rounding)
}

/// (x << offset) / y
#[inline]
fn shl_div(x: u128, y: u128, offset: u8, rounding: Rounding) -> Option<u128> {
    let scale = 1u128.checked_shl(offset.into())?;
    mul_div(x, scale, y, rounding)
}


 
/// 计算 (a * b) >> 64，Q64.64乘法
pub fn q64_mul(a: u128, b: u128) -> u128 {
    mul_shr(a, b, SCALE_OFFSET, Rounding::Down).unwrap_or(0)
}



/// 计算 (a << 64) / b，Q64.64除法
pub fn q64_div(a: u128, b: u128) -> u128 {
    shl_div(a, b, SCALE_OFFSET, Rounding::Down).unwrap_or(0)
}

/// 将基点转换为Q64.64格式
pub fn bps_to_q64(bps: u64) -> u128 {
    // bps / BASE_BPS 转换为Q64.64
    // 使用mul_div: (bps * 2^64) / BASE_BPS
    mul_div(bps as u128, ONE, BASE_BPS as u128, Rounding::Down).unwrap_or(0)
}


 //DLMM 计算fee 使用
#[inline]
pub fn safe_mul_shr_cast(
    x: u128,
    y: u128,
    offset: u8,
    rounding: Rounding,
) -> u128 {
    mul_shr(x, y, offset, rounding).unwrap_or(0)
}

#[inline]
pub fn safe_shl_div_cast(
    x: u128,
    y: u128,
    offset: u8,
    rounding: Rounding,
) -> u128 {
    shl_div(x, y, offset, rounding).unwrap_or(0)
}

pub fn safe_mul_div_cast(
    x: u128,
    y: u128,
    denominator: u128,
    rounding: Rounding,
) -> u128 {
    mul_div(x, y, denominator, rounding).unwrap_or(0)
}



/// (x * y) / denominator 
pub fn mul_div_u256(x: U256, y: U256, denominator: U256, rounding: Rounding) -> Option<U256> {
    if denominator == U256::ZERO {
        return None;
    }

    let x = U512::from(x);
    let y = U512::from(y);
    let denominator = U512::from(denominator);

    let prod = x.checked_mul(y)?;

    let result = match rounding {
        Rounding::Up => prod.div_ceil(denominator),
        Rounding::Down => {
            let (quotient, _) = prod.div_rem(denominator);
            quotient
        }
    };
    if result > U512::from(U256::MAX) {
        None
    } else {
        Some(U256::from(result))
    }
}

/// 计算u128的整数平方根，使用牛顿迭代法
/// 返回最大的整数x使得x² ≤ n
pub fn integer_sqrt_u128(n: u128) -> u128 {
    if n == 0 {
        return 0;
    }
    if n <= 1 {
        return n;
    }

    // 初始猜测：使用位操作快速估算
    let mut x = 1u128 << ((128 - n.leading_zeros() + 1) / 2);
    
    // 牛顿迭代法：x_{n+1} = (x_n + n/x_n) / 2
    loop {
        let y = (x + n / x) / 2;
        if y >= x {
            return x;
        }
        x = y;

    }
}