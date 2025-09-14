use std::{
    cmp::Ordering,
    fmt::{Display, Formatter, Result as FmtResult},
    str::from_utf8_unchecked,
};

use crate::utils::errors::ErrorCode;

const NUM_WORDS: usize = 4;

#[derive(Copy, Clone, Debug)]
pub struct U256Muldiv {
    pub items: [u64; NUM_WORDS],
}

impl U256Muldiv {
    pub fn new(h: u128, l: u128) -> Self {
        U256Muldiv {
            items: [l.lo(), l.hi(), h.lo(), h.hi()],
        }
    }

    fn copy(&self) -> Self {
        let mut items: [u64; NUM_WORDS] = [0; NUM_WORDS];
        items.copy_from_slice(&self.items);
        U256Muldiv { items }
    }

    fn update_word(&mut self, index: usize, value: u64) {
        self.items[index] = value;
    }

    fn num_words(&self) -> usize {
        for i in (0..self.items.len()).rev() {
            if self.items[i] != 0 {
                return i + 1;
            }
        }
        0
    }

    pub fn get_word(&self, index: usize) -> u64 {
        self.items[index]
    }

    pub fn get_word_u128(&self, index: usize) -> u128 {
        self.items[index] as u128
    }

    // Logical-left shift, does not trigger overflow
    pub fn shift_word_left(&self) -> Self {
        let mut result = U256Muldiv::new(0, 0);

        for i in (0..NUM_WORDS - 1).rev() {
            result.items[i + 1] = self.items[i];
        }

        result
    }

    pub fn checked_shift_word_left(&self) -> Option<Self> {
        let last_element = self.items.last();

        match last_element {
            None => Some(self.shift_word_left()),
            Some(element) => {
                if *element > 0 {
                    None
                } else {
                    Some(self.shift_word_left())
                }
            }
        }
    }

    // Logical-left shift, does not trigger overflow
    pub fn shift_left(&self, mut shift_amount: u32) -> Self {
        // Return 0 if shift is greater than number of bits
        if shift_amount >= U64_RESOLUTION * (NUM_WORDS as u32) {
            return U256Muldiv::new(0, 0);
        }

        let mut result = self.copy();

        while shift_amount >= U64_RESOLUTION {
            result = result.shift_word_left();
            shift_amount -= U64_RESOLUTION;
        }

        if shift_amount == 0 {
            return result;
        }

        for i in (1..NUM_WORDS).rev() {
            result.items[i] = (result.items[i] << shift_amount)
                | (result.items[i - 1] >> (U64_RESOLUTION - shift_amount));
        }

        result.items[0] <<= shift_amount;

        result
    }

    // Logical-right shift, does not trigger overflow
    pub fn shift_word_right(&self) -> Self {
        let mut result = U256Muldiv::new(0, 0);

        for i in 0..NUM_WORDS - 1 {
            result.items[i] = self.items[i + 1]
        }

        result
    }

    // Logical-right shift, does not trigger overflow
    pub fn shift_right(&self, mut shift_amount: u32) -> Self {
        // Return 0 if shift is greater than number of bits
        if shift_amount >= U64_RESOLUTION * (NUM_WORDS as u32) {
            return U256Muldiv::new(0, 0);
        }

        let mut result = self.copy();

        while shift_amount >= U64_RESOLUTION {
            result = result.shift_word_right();
            shift_amount -= U64_RESOLUTION;
        }

        if shift_amount == 0 {
            return result;
        }

        for i in 0..NUM_WORDS - 1 {
            result.items[i] = (result.items[i] >> shift_amount)
                | (result.items[i + 1] << (U64_RESOLUTION - shift_amount));
        }

        result.items[3] >>= shift_amount;

        result
    }

    #[allow(clippy::should_implement_trait)]
    pub fn eq(&self, other: U256Muldiv) -> bool {
        for i in 0..self.items.len() {
            if self.items[i] != other.items[i] {
                return false;
            }
        }

        true
    }

    pub fn lt(&self, other: U256Muldiv) -> bool {
        for i in (0..self.items.len()).rev() {
            match self.items[i].cmp(&other.items[i]) {
                Ordering::Less => return true,
                Ordering::Greater => return false,
                Ordering::Equal => {}
            }
        }

        false
    }

    pub fn gt(&self, other: U256Muldiv) -> bool {
        for i in (0..self.items.len()).rev() {
            match self.items[i].cmp(&other.items[i]) {
                Ordering::Less => return false,
                Ordering::Greater => return true,
                Ordering::Equal => {}
            }
        }

        false
    }

    pub fn lte(&self, other: U256Muldiv) -> bool {
        for i in (0..self.items.len()).rev() {
            match self.items[i].cmp(&other.items[i]) {
                Ordering::Less => return true,
                Ordering::Greater => return false,
                Ordering::Equal => {}
            }
        }

        true
    }

    pub fn gte(&self, other: U256Muldiv) -> bool {
        for i in (0..self.items.len()).rev() {
            match self.items[i].cmp(&other.items[i]) {
                Ordering::Less => return false,
                Ordering::Greater => return true,
                Ordering::Equal => {}
            }
        }

        true
    }

    pub fn try_into_u128(&self) -> Result<u128, ErrorCode> {
        if self.num_words() > 2 {
            return Err(ErrorCode::CalculateOverflow.into());
        }

        Ok(((self.items[1] as u128) << U64_RESOLUTION) | (self.items[0] as u128))
    }

    pub fn is_zero(self) -> bool {
        for i in 0..NUM_WORDS {
            if self.items[i] != 0 {
                return false;
            }
        }

        true
    }

    // Input:
    //  m = U256::MAX + 1 (which is the amount used for overflow)
    //  n = input value
    // Output:
    //  r = smallest positive additive inverse of n mod m
    //
    // We wish to find r, s.t., r + n ≡ 0 mod m;
    // We generally wish to find this r since r ≡ -n mod m
    // and can make operations with n with large number of bits
    // fit into u256 space without overflow
    pub fn get_add_inverse(&self) -> Self {
        // Additive inverse of 0 is 0
        if self.eq(U256Muldiv::new(0, 0)) {
            return U256Muldiv::new(0, 0);
        }
        // To ensure we don't overflow, we begin with max and do a subtraction
        U256Muldiv::new(u128::MAX, u128::MAX)
            .sub(*self)
            .add(U256Muldiv::new(0, 1))
    }

    // Result overflows if the result is greater than 2^256-1
    pub fn add(&self, other: U256Muldiv) -> Self {
        let mut result = U256Muldiv::new(0, 0);

        let mut carry = 0;
        for i in 0..NUM_WORDS {
            let x = self.get_word_u128(i);
            let y = other.get_word_u128(i);
            let t = x + y + carry;
            result.update_word(i, t.lo());

            carry = t.hi_u128();
        }

        result
    }

    // Result underflows if the result is greater than 2^256-1
    pub fn sub(&self, other: U256Muldiv) -> Self {
        let mut result = U256Muldiv::new(0, 0);

        let mut carry = 0;
        for i in 0..NUM_WORDS {
            let x = self.get_word(i);
            let y = other.get_word(i);
            let (t0, overflowing0) = x.overflowing_sub(y);
            let (t1, overflowing1) = t0.overflowing_sub(carry);
            result.update_word(i, t1);

            carry = if overflowing0 || overflowing1 { 1 } else { 0 };
        }

        result
    }

    // Result overflows if great than 2^256-1
    pub fn mul(&self, other: U256Muldiv) -> Self {
        let mut result = U256Muldiv::new(0, 0);

        let m = self.num_words();
        let n = other.num_words();

        for j in 0..n {
            let mut k = 0;
            for i in 0..m {
                let x = self.get_word_u128(i);
                let y = other.get_word_u128(j);
                if i + j < NUM_WORDS {
                    let z = result.get_word_u128(i + j);
                    let t = x.wrapping_mul(y).wrapping_add(z).wrapping_add(k);
                    result.update_word(i + j, t.lo());
                    k = t.hi_u128();
                }
            }

            // Don't update the carry word
            if j + m < NUM_WORDS {
                result.update_word(j + m, k as u64);
            }
        }

        result
    }

    // Result returns 0 if divide by zero
    pub fn div(&self, mut divisor: U256Muldiv, return_remainder: bool) -> (Self, Self) {
        let mut dividend = self.copy();
        let mut quotient = U256Muldiv::new(0, 0);

        let num_dividend_words = dividend.num_words();
        let num_divisor_words = divisor.num_words();

        if num_divisor_words == 0 {
            panic!("divide by zero");
        }

        // Case 0. If either the dividend or divisor is 0, return 0
        if num_dividend_words == 0 {
            return (U256Muldiv::new(0, 0), U256Muldiv::new(0, 0));
        }

        // Case 1. Dividend is smaller than divisor, quotient = 0, remainder = dividend
        if num_dividend_words < num_divisor_words {
            if return_remainder {
                return (U256Muldiv::new(0, 0), dividend);
            } else {
                return (U256Muldiv::new(0, 0), U256Muldiv::new(0, 0));
            }
        }

        // Case 2. Dividend is smaller than u128, divisor <= dividend, perform math in u128 space
        if num_dividend_words < 3 {
            let dividend = dividend.try_into_u128().unwrap();
            let divisor = divisor.try_into_u128().unwrap();
            let quotient = dividend / divisor;
            if return_remainder {
                let remainder = dividend % divisor;
                return (U256Muldiv::new(0, quotient), U256Muldiv::new(0, remainder));
            } else {
                return (U256Muldiv::new(0, quotient), U256Muldiv::new(0, 0));
            }
        }

        // Case 3. Divisor is single-word, we must isolate this case for correctness
        if num_divisor_words == 1 {
            let mut k = 0;
            for j in (0..num_dividend_words).rev() {
                let d1 = hi_lo(k.lo(), dividend.get_word(j));
                let d2 = divisor.get_word_u128(0);
                let q = d1 / d2;
                k = d1 - d2 * q;
                quotient.update_word(j, q.lo());
            }

            if return_remainder {
                return (quotient, U256Muldiv::new(0, k));
            } else {
                return (quotient, U256Muldiv::new(0, 0));
            }
        }

        // Normalize the division by shifting left
        let s = divisor.get_word(num_divisor_words - 1).leading_zeros();
        let b = dividend.get_word(num_dividend_words - 1).leading_zeros();

        // Conditional carry space for normalized division
        let mut dividend_carry_space: u64 = 0;
        if num_dividend_words == NUM_WORDS && b < s {
            dividend_carry_space = dividend.items[num_dividend_words - 1] >> (U64_RESOLUTION - s);
        }
        dividend = dividend.shift_left(s);
        divisor = divisor.shift_left(s);

        for j in (0..num_dividend_words - num_divisor_words + 1).rev() {
            let result = div_loop(
                j,
                num_divisor_words,
                dividend,
                &mut dividend_carry_space,
                divisor,
                quotient,
            );
            quotient = result.0;
            dividend = result.1;
        }

        if return_remainder {
            dividend = dividend.shift_right(s);
            (quotient, dividend)
        } else {
            (quotient, U256Muldiv::new(0, 0))
        }
    }
}

impl Display for U256Muldiv {
    fn fmt(&self, f: &mut Formatter) -> FmtResult {
        let mut buf = [0_u8; NUM_WORDS * 20];
        let mut i = buf.len() - 1;

        let ten = U256Muldiv::new(0, 10);
        let mut current = *self;

        loop {
            let (quotient, remainder) = current.div(ten, true);
            let digit = remainder.get_word(0) as u8;
            buf[i] = digit + b'0';
            current = quotient;

            if current.is_zero() {
                break;
            }

            i -= 1;
        }

        let s = unsafe { from_utf8_unchecked(&buf[i..]) };

        f.write_str(s)
    }
}

const U64_MAX: u128 = u64::MAX as u128;
const U64_RESOLUTION: u32 = 64;

pub trait LoHi {
    fn lo(self) -> u64;
    fn hi(self) -> u64;
    fn lo_u128(self) -> u128;
    fn hi_u128(self) -> u128;
}

impl LoHi for u128 {
    fn lo(self) -> u64 {
        (self & U64_MAX) as u64
    }
    fn lo_u128(self) -> u128 {
        self & U64_MAX
    }
    fn hi(self) -> u64 {
        (self >> U64_RESOLUTION) as u64
    }
    fn hi_u128(self) -> u128 {
        self >> U64_RESOLUTION
    }
}

pub fn hi_lo(hi: u64, lo: u64) -> u128 {
    ((hi as u128) << U64_RESOLUTION) | (lo as u128)
}

pub fn mul_u256(v: u128, n: u128) -> U256Muldiv {
    // do 128 bits multiply
    //                   nh   nl
    //                *  vh   vl
    //                ----------
    // a0 =              vl * nl
    // a1 =         vl * nh
    // b0 =         vh * nl
    // b1 =  + vh * nh
    //       -------------------
    //        c1h  c1l  c0h  c0l
    //
    // "a0" is optimized away, result is stored directly in c0.  "b1" is
    // optimized away, result is stored directly in c1.
    //

    let mut c0 = v.lo_u128() * n.lo_u128();
    let a1 = v.lo_u128() * n.hi_u128();
    let b0 = v.hi_u128() * n.lo_u128();

    // add the high word of a0 to the low words of a1 and b0 using c1 as
    // scrach space to capture the carry.  the low word of the result becomes
    // the final high word of c0
    let mut c1 = c0.hi_u128() + a1.lo_u128() + b0.lo_u128();

    c0 = hi_lo(c1.lo(), c0.lo());

    // add the carry from the result above (found in the high word of c1) and
    // the high words of a1 and b0 to b1, the result is c1.
    c1 = v.hi_u128() * n.hi_u128() + c1.hi_u128() + a1.hi_u128() + b0.hi_u128();

    U256Muldiv::new(c1, c0)
}

fn div_loop(
    index: usize,
    num_divisor_words: usize,
    mut dividend: U256Muldiv,
    dividend_carry_space: &mut u64,
    divisor: U256Muldiv,
    mut quotient: U256Muldiv,
) -> (U256Muldiv, U256Muldiv) {
    let use_carry = (index + num_divisor_words) == NUM_WORDS;
    let div_hi = if use_carry {
        *dividend_carry_space
    } else {
        dividend.get_word(index + num_divisor_words)
    };
    let d0 = hi_lo(div_hi, dividend.get_word(index + num_divisor_words - 1));
    let d1 = divisor.get_word_u128(num_divisor_words - 1);

    let mut qhat = d0 / d1;
    let mut rhat = d0 - d1 * qhat;

    let d0_2 = dividend.get_word(index + num_divisor_words - 2);
    let d1_2 = divisor.get_word_u128(num_divisor_words - 2);

    let mut cmp1 = hi_lo(rhat.lo(), d0_2);
    let mut cmp2 = qhat.wrapping_mul(d1_2);

    while qhat.hi() != 0 || cmp2 > cmp1 {
        qhat -= 1;
        rhat += d1;
        if rhat.hi() != 0 {
            break;
        }

        cmp1 = hi_lo(rhat.lo(), cmp1.lo());
        cmp2 -= d1_2;
    }

    let mut k = 0;
    let mut t;
    for i in 0..num_divisor_words {
        let p = qhat * (divisor.get_word_u128(i));
        t = (dividend.get_word_u128(index + i))
            .wrapping_sub(k)
            .wrapping_sub(p.lo_u128());
        dividend.update_word(index + i, t.lo());
        k = ((p >> U64_RESOLUTION) as u64).wrapping_sub((t >> U64_RESOLUTION) as u64) as u128;
    }

    let d_head = if use_carry {
        *dividend_carry_space as u128
    } else {
        dividend.get_word_u128(index + num_divisor_words)
    };

    t = d_head.wrapping_sub(k);
    if use_carry {
        *dividend_carry_space = t.lo();
    } else {
        dividend.update_word(index + num_divisor_words, t.lo());
    }

    if k > d_head {
        qhat -= 1;
        k = 0;
        for i in 0..num_divisor_words {
            t = dividend
                .get_word_u128(index + i)
                .wrapping_add(divisor.get_word_u128(i))
                .wrapping_add(k);
            dividend.update_word(index + i, t.lo());
            k = t >> U64_RESOLUTION;
        }

        let new_carry = dividend
            .get_word_u128(index + num_divisor_words)
            .wrapping_add(k)
            .lo();
        if use_carry {
            *dividend_carry_space = new_carry
        } else {
            dividend.update_word(
                index + num_divisor_words,
                dividend
                    .get_word_u128(index + num_divisor_words)
                    .wrapping_add(k)
                    .lo(),
            );
        }
    }

    quotient.update_word(index, qhat.lo());

    (quotient, dividend)
}




pub const Q64_RESOLUTION: u8 = 64;
pub const Q64_MASK: u128 = 0xFFFF_FFFF_FFFF_FFFF;
pub const TO_Q64: u128 = 1u128 << Q64_RESOLUTION;
pub const MAX_SQRT_PRICE_X64: u128 = 79226673515401279992447579055;
pub const MIN_SQRT_PRICE_X64: u128 = 4295048016;
// use super::{
//     div_round_up_if, div_round_up_if_u256, mul_u256, U256Muldiv, MAX_SQRT_PRICE_X64,
//     MIN_SQRT_PRICE_X64,
// };



pub fn div_round_up_if_u256(
    n: U256Muldiv,
    d: U256Muldiv,
    round_up: bool,
) -> Result<u128, ErrorCode> {
    let (quotient, remainder) = n.div(d, round_up);

    let result = if round_up && !remainder.is_zero() {
        quotient.add(U256Muldiv::new(0, 1))
    } else {
        quotient
    };

    result.try_into_u128()
}

pub fn div_round_up_if(n: u128, d: u128, round_up: bool) -> Result<u128, ErrorCode> {
    if d == 0 {
        return Err(ErrorCode::DivideByZero);
    }

    let q = n / d;

    Ok(if round_up && n % d > 0 { q + 1 } else { q })
}

// Fee rate is represented as hundredths of a basis point.
// Fee amount = total_amount * fee_rate / 1_000_000.
// Max fee rate supported is 6%.
pub const MAX_FEE_RATE: u16 = 60_000;

// Assuming that FEE_RATE is represented as hundredths of a basis point
// We want FEE_RATE_MUL_VALUE = 1/FEE_RATE_UNIT, so 1e6
pub const FEE_RATE_MUL_VALUE: u128 = 1_000_000;

// Protocol fee rate is represented as a basis point.
// Protocol fee amount = fee_amount * protocol_fee_rate / 10_000.
// Max protocol fee rate supported is 25% of the fee rate.
pub const MAX_PROTOCOL_FEE_RATE: u16 = 2_500;

// Assuming that PROTOCOL_FEE_RATE is represented as a basis point
// We want PROTOCOL_FEE_RATE_MUL_VALUE = 1/PROTOCOL_FEE_UNIT, so 1e4
pub const PROTOCOL_FEE_RATE_MUL_VALUE: u128 = 10_000;

#[derive(Debug)]
pub enum AmountDeltaU64 {
    Valid(u64),
    ExceedsMax(ErrorCode),
}

impl AmountDeltaU64 {
    pub fn lte(&self, other: u64) -> bool {
        match self {
            AmountDeltaU64::Valid(value) => *value <= other,
            AmountDeltaU64::ExceedsMax(_) => false,
        }
    }

    pub fn exceeds_max(&self) -> bool {
        match self {
            AmountDeltaU64::Valid(_) => false,
            AmountDeltaU64::ExceedsMax(_) => true,
        }
    }

    pub fn value(self) -> u64 {
        match self {
            AmountDeltaU64::Valid(value) => value,
            // This should never happen
            AmountDeltaU64::ExceedsMax(_) => panic!("Called unwrap on AmountDeltaU64::ExceedsMax"),
        }
    }
}

//
// Get change in token_a corresponding to a change in price
//

// 6.16
// Δt_a = Δ(1 / sqrt_price) * liquidity

// Replace delta
// Δt_a = (1 / sqrt_price_upper - 1 / sqrt_price_lower) * liquidity

// Common denominator to simplify
// Δt_a = ((sqrt_price_lower - sqrt_price_upper) / (sqrt_price_upper * sqrt_price_lower)) * liquidity

// Δt_a = (liquidity * (sqrt_price_lower - sqrt_price_upper)) / (sqrt_price_upper * sqrt_price_lower)
pub fn get_amount_delta_a(
    sqrt_price_0: u128,
    sqrt_price_1: u128,
    liquidity: u128,
    round_up: bool,
) -> Result<u64, ErrorCode> {
    match try_get_amount_delta_a(sqrt_price_0, sqrt_price_1, liquidity, round_up) {
        Ok(AmountDeltaU64::Valid(value)) => Ok(value),
        Ok(AmountDeltaU64::ExceedsMax(error)) => Err(error),
        Err(error) => Err(error),
    }
}

pub fn try_get_amount_delta_a(
    sqrt_price_0: u128,
    sqrt_price_1: u128,
    liquidity: u128,
    round_up: bool,
) -> Result<AmountDeltaU64, ErrorCode> {
    let (sqrt_price_lower, sqrt_price_upper) = increasing_price_order(sqrt_price_0, sqrt_price_1);

    let sqrt_price_diff = sqrt_price_upper - sqrt_price_lower;

    let numerator = mul_u256(liquidity, sqrt_price_diff)
        .checked_shift_word_left()
        .ok_or(ErrorCode::CalculateOverflow)?;

    let denominator = mul_u256(sqrt_price_upper, sqrt_price_lower);

    let (quotient, remainder) = numerator.div(denominator, round_up);

    let result = if round_up && !remainder.is_zero() {
        quotient.add(U256Muldiv::new(0, 1)).try_into_u128()
    } else {
        quotient.try_into_u128()
    };

    match result {
        Ok(result) => {
            if result > u64::MAX as u128 {
                return Ok(AmountDeltaU64::ExceedsMax(ErrorCode::MaxTokenOverflow));
            }

            Ok(AmountDeltaU64::Valid(result as u64))
        }
        Err(err) => Ok(AmountDeltaU64::ExceedsMax(err)),
    }
}

//
// Get change in token_b corresponding to a change in price
//

// 6.14
// Δt_b = Δ(sqrt_price) * liquidity

// Replace delta
// Δt_b = (sqrt_price_upper - sqrt_price_lower) * liquidity
pub fn get_amount_delta_b(
    sqrt_price_0: u128,
    sqrt_price_1: u128,
    liquidity: u128,
    round_up: bool,
) -> Result<u64, ErrorCode> {
    match try_get_amount_delta_b(sqrt_price_0, sqrt_price_1, liquidity, round_up) {
        Ok(AmountDeltaU64::Valid(value)) => Ok(value),
        Ok(AmountDeltaU64::ExceedsMax(error)) => Err(error),
        Err(error) => Err(error),
    }
}

pub fn try_get_amount_delta_b(
    sqrt_price_0: u128,
    sqrt_price_1: u128,
    liquidity: u128,
    round_up: bool,
) -> Result<AmountDeltaU64, ErrorCode> {
    let (sqrt_price_lower, sqrt_price_upper) = increasing_price_order(sqrt_price_0, sqrt_price_1);

    // customized checked_mul_shift_right_round_up_if

    let n0 = liquidity;
    let n1 = sqrt_price_upper - sqrt_price_lower;

    if n0 == 0 || n1 == 0 {
        return Ok(AmountDeltaU64::Valid(0));
    }

    if let Some(p) = n0.checked_mul(n1) {
        let result = (p >> Q64_RESOLUTION) as u64;

        let should_round = round_up && (p & Q64_MASK > 0);
        if should_round && result == u64::MAX {
            return Ok(AmountDeltaU64::ExceedsMax(
                ErrorCode::CalculateOverflow,
            ));
        }

        Ok(AmountDeltaU64::Valid(if should_round {
            result + 1
        } else {
            result
        }))
    } else {
        Ok(AmountDeltaU64::ExceedsMax(
            ErrorCode::CalculateOverflow,
        ))
    }
}

pub fn increasing_price_order(sqrt_price_0: u128, sqrt_price_1: u128) -> (u128, u128) {
    if sqrt_price_0 > sqrt_price_1 {
        (sqrt_price_1, sqrt_price_0)
    } else {
        (sqrt_price_0, sqrt_price_1)
    }
}

//
// Get change in price corresponding to a change in token_a supply
//
// 6.15
// Δ(1 / sqrt_price) = Δt_a / liquidity
//
// Replace delta
// 1 / sqrt_price_new - 1 / sqrt_price = amount / liquidity
//
// Move sqrt price to other side
// 1 / sqrt_price_new = (amount / liquidity) + (1 / sqrt_price)
//
// Common denominator for right side
// 1 / sqrt_price_new = (sqrt_price * amount + liquidity) / (sqrt_price * liquidity)
//
// Invert fractions
// sqrt_price_new = (sqrt_price * liquidity) / (liquidity + amount * sqrt_price)
pub fn get_next_sqrt_price_from_a_round_up(
    sqrt_price: u128,
    liquidity: u128,
    amount: u64,
    amount_specified_is_input: bool,
) -> Result<u128, ErrorCode> {
    if amount == 0 {
        return Ok(sqrt_price);
    }
    let product = mul_u256(sqrt_price, amount as u128);

    let numerator = mul_u256(liquidity, sqrt_price)
        .checked_shift_word_left()
        .ok_or(ErrorCode::CalculateOverflow)?;

    // In this scenario the denominator will end up being < 0
    let liquidity_shift_left = U256Muldiv::new(0, liquidity).shift_word_left();
    if !amount_specified_is_input && liquidity_shift_left.lte(product) {
        return Err(ErrorCode::DivideByZero);
    }

    let denominator = if amount_specified_is_input {
        liquidity_shift_left.add(product)
    } else {
        liquidity_shift_left.sub(product)
    };

    let price = div_round_up_if_u256(numerator, denominator, true)?;
    if price < MIN_SQRT_PRICE_X64 {
        return Err(ErrorCode::InvalidSqrtPrice);
    } else if price > MAX_SQRT_PRICE_X64 {
        return Err(ErrorCode::InvalidSqrtPrice);
    }

    Ok(price)
}

//
// Get change in price corresponding to a change in token_b supply
//
// 6.13
// Δ(sqrt_price) = Δt_b / liquidity
pub fn get_next_sqrt_price_from_b_round_down(
    sqrt_price: u128,
    liquidity: u128,
    amount: u64,
    amount_specified_is_input: bool,
) -> Result<u128, ErrorCode> {
    // We always want square root price to be rounded down, which means
    // Case 3. If we are fixing input (adding B), we are increasing price, we want delta to be floor(delta)
    // sqrt_price + floor(delta) < sqrt_price + delta
    //
    // Case 4. If we are fixing output (removing B), we are decreasing price, we want delta to be ceil(delta)
    // sqrt_price - ceil(delta) < sqrt_price - delta

    // Q64.0 << 64 => Q64.64
    let amount_x64 = (amount as u128) << Q64_RESOLUTION;

    // Q64.64 / Q64.0 => Q64.64
    let delta = div_round_up_if(amount_x64, liquidity, !amount_specified_is_input)?;

    // Q64(32).64 +/- Q64.64
    if amount_specified_is_input {
        // We are adding token b to supply, causing price to increase
        sqrt_price
            .checked_add(delta)
            .ok_or(ErrorCode::InvalidSqrtPrice)
    } else {
        // We are removing token b from supply,. causing price to decrease
        sqrt_price
            .checked_sub(delta)
            .ok_or(ErrorCode::InvalidSqrtPrice)
    }
}

pub fn get_next_sqrt_price(
    sqrt_price: u128,
    liquidity: u128,
    amount: u64,
    amount_specified_is_input: bool,
    a_to_b: bool,
) -> Result<u128, ErrorCode> {
    if amount_specified_is_input == a_to_b {
        // We are fixing A
        // Case 1. amount_specified_is_input = true, a_to_b = true
        // We are exchanging A to B with at most _amount_ of A (input)
        //
        // Case 2. amount_specified_is_input = false, a_to_b = false
        // We are exchanging B to A wanting to guarantee at least _amount_ of A (output)
        //
        // In either case we want the sqrt_price to be rounded up.
        //
        // Eq 1. sqrt_price = sqrt( b / a )
        //
        // Case 1. amount_specified_is_input = true, a_to_b = true
        // We are adding token A to the supply, causing price to decrease (Eq 1.)
        // Since we are fixing input, we can not exceed the amount that is being provided by the user.
        // Because a higher price is inversely correlated with an increased supply of A,
        // a higher price means we are adding less A. Thus when performing math, we wish to round the
        // price up, since that means that we are guaranteed to not exceed the fixed amount of A provided.
        //
        // Case 2. amount_specified_is_input = false, a_to_b = false
        // We are removing token A from the supply, causing price to increase (Eq 1.)
        // Since we are fixing output, we want to guarantee that the user is provided at least _amount_ of A
        // Because a higher price is correlated with a decreased supply of A,
        // a higher price means we are removing more A to give to the user. Thus when performing math, we wish
        // to round the price up, since that means we guarantee that user receives at least _amount_ of A
        get_next_sqrt_price_from_a_round_up(
            sqrt_price,
            liquidity,
            amount,
            amount_specified_is_input,
        )
    } else {
        // We are fixing B
        // Case 3. amount_specified_is_input = true, a_to_b = false
        // We are exchanging B to A using at most _amount_ of B (input)
        //
        // Case 4. amount_specified_is_input = false, a_to_b = true
        // We are exchanging A to B wanting to guarantee at least _amount_ of B (output)
        //
        // In either case we want the sqrt_price to be rounded down.
        //
        // Eq 1. sqrt_price = sqrt( b / a )
        //
        // Case 3. amount_specified_is_input = true, a_to_b = false
        // We are adding token B to the supply, causing price to increase (Eq 1.)
        // Since we are fixing input, we can not exceed the amount that is being provided by the user.
        // Because a lower price is inversely correlated with an increased supply of B,
        // a lower price means that we are adding less B. Thus when performing math, we wish to round the
        // price down, since that means that we are guaranteed to not exceed the fixed amount of B provided.
        //
        // Case 4. amount_specified_is_input = false, a_to_b = true
        // We are removing token B from the supply, causing price to decrease (Eq 1.)
        // Since we are fixing output, we want to guarantee that the user is provided at least _amount_ of B
        // Because a lower price is correlated with a decreased supply of B,
        // a lower price means we are removing more B to give to the user. Thus when performing math, we
        // wish to round the price down, since that means we guarantee that the user receives at least _amount_ of B
        get_next_sqrt_price_from_b_round_down(
            sqrt_price,
            liquidity,
            amount,
            amount_specified_is_input,
        )
    }
}
 