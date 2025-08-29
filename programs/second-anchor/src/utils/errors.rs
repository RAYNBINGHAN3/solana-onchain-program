use anchor_lang::prelude::*;
/// 错误代码定义
#[error_code]
pub enum ErrorCode {
    #[msg("Invalid pool owner")]
    InvalidPoolOwner,
    
    #[msg("Token address mismatch")]
    TokenMismatch,
    
    #[msg("Zero liquidity")]
    ZeroLiquidity,
    
    #[msg("Math overflow")]
    MathOverflow,

    #[msg("Zero price")]
    ZeroPrice,

    #[msg("Invalid account")]
    InvalidAccount,

    #[msg("Invalid token pair")]
    InvalidTokenPair,

    #[msg("Bin ID out of range")]
    BinIdOutOfRange,

    #[msg("No profit")]
    NoProfit,

    #[msg("Wrong calculation")]
    WrongCalculation,

    #[msg("Invalid fee scheduler mode")]
    InvalidFeeSchedulerMode,

    #[msg("Invalid activation type")]
    InvalidActivationType,

    #[msg("Invalid collect fee mode")]
    InvalidCollectFeeMode,


    None,
}