# Pump AMM 实现总结

## 概述

基于 Pump Fun SDK 和 IDL 文件，完成了 Pump AMM 的数据解析、手续费计算、价格计算和交易报价功能的实现。

## 核心功能

### 1. 数据结构解析

#### PumpPool 结构体
```rust
pub struct PumpPool {
    pub pool_bump: u8,
    pub index: u16,
    pub creator: Pubkey,
    pub base_mint: Pubkey,
    pub quote_mint: Pubkey,
    pub lp_mint: Pubkey,
    pub pool_base_token_account: Pubkey,
    pub pool_quote_token_account: Pubkey,
    pub lp_supply: u64,
    pub coin_creator: Pubkey,
}
```

#### PumpGlobalConfig 结构体
```rust
pub struct PumpGlobalConfig {
    pub admin: Pubkey,
    pub lp_fee_basis_points: u64,        // LP 手续费 (基点)
    pub protocol_fee_basis_points: u64,  // 协议手续费 (基点) 
    pub disable_flags: u8,
    pub protocol_fee_recipients: [Pubkey; 8],
    pub coin_creator_fee_basis_points: u64, // 代币创建者手续费 (基点)
    pub admin_set_coin_creator_authority: Pubkey,
}
```

### 2. 手续费计算

基于 SDK 分析，Pump AMM 有三种手续费：

- **LP 手续费**: `lp_fee_basis_points` (通常 20 基点 = 0.2%)
- **协议手续费**: `protocol_fee_basis_points` (通常 5 基点 = 0.05%)  
- **代币创建者手续费**: `coin_creator_fee_basis_points` (通常 5 基点 = 0.05%)

**总手续费率**: `20 + 5 + 5 = 30 基点 = 0.3%`

### 3. 价格计算

使用恒定乘积公式 (x * y = k)：

```rust
// 价格 = quote_reserve / base_reserve (Q64.64 格式)
let price = safe_mul_div_cast(
    pool_state.quote_reserve as u128,
    ONE as u128,
    pool_state.base_reserve as u128,
    Rounding::Down,
);
```

### 4. 交易报价函数

#### 买入计算 (SOL -> Token)
```rust
pub fn pump_quote_exact_input_wsol(
    wsol_amount: u64,
    slippage_bps: u64,
    pool_state: &PumpPoolState,
) -> Result<(u64, u64)> // (token_amount_out, max_wsol_in)
```

**计算逻辑**:
1. 计算各种手续费：`lp_fee + protocol_fee + coin_creator_fee`
2. 实际交易金额：`effective_wsol = wsol_amount - total_fees`
3. 输出代币数量：`token_out = base_reserve * effective_wsol / (quote_reserve + effective_wsol)`
4. 考虑滑点的最大输入：`max_wsol_in = wsol_amount * (1 + slippage)`

#### 卖出计算 (Token -> SOL)
```rust
pub fn pump_quote_exact_input_token(
    token_amount: u64,
    slippage_bps: u64,
    pool_state: &PumpPoolState,
) -> Result<(u64, u64)> // (wsol_amount_out, min_wsol_out)
```

**计算逻辑**:
1. 计算原始输出：`wsol_out = quote_reserve * token_amount / (base_reserve + token_amount)`
2. 扣除各种手续费：`final_wsol = wsol_out - total_fees`
3. 考虑滑点的最小输出：`min_wsol_out = final_wsol * (1 - slippage)`

## SDK 对比分析

### 买入逻辑对比

**SDK buyBaseInputInternal**:
```javascript
// 计算所需的 quote 数量
const quoteAmountIn = ceilDiv(numerator, denominator);
// 计算各种手续费
const lpFee = fee(quoteAmountIn, globalConfig.lpFeeBasisPoints);
const protocolFee = fee(quoteAmountIn, globalConfig.protocolFeeBasisPoints);
const coinCreatorFee = fee(quoteAmountIn, globalConfig.coinCreatorFeeBasisPoints);
// 总费用
const totalQuote = quoteAmountIn.add(lpFee).add(protocolFee).add(coinCreatorFee);
```

**我们的实现**:
```rust
// 先计算手续费，再计算有效交易金额
let lp_fee = ceil_div(wsol_amount * config.lp_fee_basis_points, 10000);
let protocol_fee = ceil_div(wsol_amount * config.protocol_fee_basis_points, 10000);
let coin_creator_fee = ceil_div(wsol_amount * config.coin_creator_fee_basis_points, 10000);
let effective_wsol = wsol_amount.checked_sub(total_fees)?;
```

### 卖出逻辑对比

**SDK sellBaseInputInternal**:
```javascript
// 计算原始输出
const quoteAmountOut = quoteReserve.mul(base).div(baseReserve.add(base));
// 扣除手续费
const finalQuote = quoteAmountOut.sub(lpFee).sub(protocolFee).sub(coinCreatorFee);
```

**我们的实现**:
```rust
// 计算原始输出
let wsol_before_fees = (numerator / denominator) as u64;
// 扣除手续费
let wsol_amount_out = wsol_before_fees.checked_sub(total_fees)?;
```

## 集成到价格比较系统

1. **添加 PUMP 变体**到 `ParsedPoolState` 枚举
2. **更新解析函数**支持 PUMP 池数据解析
3. **集成价格计算**到套利分析系统
4. **支持多池价格比较**，包括 CPMM、DLMM、DAMMV2 和 PUMP

## 使用示例

```rust
// 创建 PUMP 池状态
let mut pool_state = PumpPoolState { /* ... */ };

// 解析池数据
parse_pump_pool_data(&pool_index, wsol_mint, token_mint, accounts, &mut pool_state)?;

// 计算价格
calculate_pump_price(&mut pool_state, wsol_mint)?;

// 买入报价 (1 SOL, 1% 滑点)
let (token_out, max_wsol_in) = pump_quote_exact_input_wsol(
    1_000_000_000, // 1 SOL
    100,           // 1% 滑点 
    &pool_state
)?;

// 卖出报价 (1000 tokens, 1% 滑点)
let (wsol_out, min_wsol_out) = pump_quote_exact_input_token(
    1000_000_000,  // 1000 tokens
    100,           // 1% 滑点
    &pool_state
)?;
```

## 技术特点

1. **精确计算**: 基于 SDK 的数学逻辑，确保计算准确性
2. **高效解析**: 使用 `bytemuck` 进行零拷贝数据解析
3. **错误处理**: 完善的错误处理和边界检查
4. **模块化设计**: 清晰的函数分离，便于测试和维护
5. **兼容性**: 与现有的价格比较系统无缝集成

## 手续费详情

基于提供的配置数据：
- LP 费用: 20 基点 (0.2%)
- 协议费用: 5 基点 (0.05%)  
- 代币创建者费用: 5 基点 (0.05%)
- **总计**: 30 基点 (0.3%)

这与 Pump.fun 的实际手续费结构一致，确保了计算的准确性。