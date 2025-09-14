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

    #[msg("Zero amount input")]
    ZeroAmountInput,

    #[msg("Invalid memo program")]
    InvalidMemoProgram,

    #[msg("Zero amount specified")]
    ZeroAmountSpecified,

    #[msg("Invalid sqrt price limit")]
    InvalidSqrtPriceLimit,

    #[msg("Too small input or output amount")]
    TooSmallInputOrOutputAmount,

    #[msg("Liquidity insufficient")]
    LiquidityInsufficient,

    #[msg("Invalid account data")]
    InvalidAccountData,

    #[msg("No valid mint found")]
    NoValidMintFound,

    #[msg("Max token overflow")]
    MaxTokenOverflow,

    #[msg("Sqrt price limit overflow")]
    SqrtPriceLimitOverflow,

    #[msg("Invalid tick index")]
    InvalidTickIndex,
    
    #[msg("Invalid tick spacing")]
    InvalidTickSpacing,

    #[msg("Invalid sqrt price")]
    InvalidSqrtPrice,

    #[msg("Calculate overflow")]
    CalculateOverflow,

    #[msg("Invalid CLMM params")]
    InvalidClmmParams,

    #[msg("Invalid tick array boundary")]
    InvalidTickArrayBoundary,

    #[msg("Divide by zero")]
    DivideByZero,

    #[msg("Invalid dyn tick")]
    InvalidDynTick,

    None,
}