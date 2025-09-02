use ruint::aliases::{U256, U512};
use crate::utils::errors::ErrorCode;
use anchor_lang::prelude::*;



/// U256 乘法除法向下舍入
pub fn mul_div_floor_u256(a: U256, b: U256, c: U256) -> Result<U256> {
    if c.is_zero() {
        return Err(ErrorCode::CalculateOverflow.into());
    }
    
    // 使用U512进行中间计算，避免溢出
    let intermediate = U512::from(a) * U512::from(b);
    let result = intermediate / U512::from(c);
    
    // 检查结果是否在U256范围内
    if result > U512::from(U256::MAX) {
        return Err(ErrorCode::MaxTokenOverflow.into());
    }
    
    // 转换回U256 - 取低256位
    let result_limbs = result.as_limbs();
    Ok(U256::from_limbs([
        result_limbs[0], result_limbs[1], result_limbs[2], result_limbs[3]
    ]))
}

/// U256 乘法除法向上舍入
pub fn mul_div_ceil_u256(a: U256, b: U256, c: U256) -> Result<U256> {
    if c.is_zero() {
        return Err(ErrorCode::CalculateOverflow.into());
    }
    
    // 使用U512进行中间计算
    let intermediate = U512::from(a) * U512::from(b);
    let quotient = intermediate / U512::from(c);
    let remainder = intermediate % U512::from(c);
    
    // 向上舍入：如果有余数，结果+1
    let result = if remainder.is_zero() {
        quotient
    } else {
        quotient + U512::from(1u8)
    };
    
    // 检查结果是否在U256范围内
    if result > U512::from(U256::MAX) {
        return Err(ErrorCode::MaxTokenOverflow.into());
    }
    
    // 转换回U256
    let result_limbs = result.as_limbs();
    Ok(U256::from_limbs([
        result_limbs[0], result_limbs[1], result_limbs[2], result_limbs[3]
    ]))
}

/// U256 除法向上舍入
pub fn div_rounding_up_u256(a: U256, b: U256) -> U256 {
    let quotient = a / b;
    let remainder = a % b;
    
    if remainder.is_zero() {
        quotient
    } else {
        quotient + U256::from(1u8)
    }
}