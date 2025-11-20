[ZH](#zh-cn) | [EN](#en)

---

<a name="en"></a>

# solana-onchain-program 

> **⚠️ This project is an early prototype by the author, with no deep performance optimization. For any production/commercial use, code refactor and optimization are strongly recommended (suggest follows `pinocchio` practices).**

---

## Project Overview

`solana-onchain-program` is a multi-DEX intelligent arbitrage (Onchain Arbitrage) contract implemented with the Solana-Anchor framework. It supports price scanning and arbitrage across major DEX pool models (e.g., CLMM, CPMM, DAMM, DLMM, PUMP, RAYDIUM, WHIRLPOOL).

### Features
- **Multi-Pool Support**: CLMM, DLMM, CPMM, DAMMV2, PUMP, Raydium, and Whirlpool.
- **Global Path Evaluation**: Scans multi-token, multi-pool prices to evaluate global arbitrage opportunities.
- **Smart Route Finding**: Automatically finds the best 2-hop and 3-hop arbitrage routes and calculates optimal trade amounts.
- **High Compatibility**: Supports all common DEXs on mainnet without hardcoding for any single protocol.
- **Extensible**: The code structure is convenient for reuse and refactoring.

### Notes
- This project is not performance-oriented and is intended for research and secondary development only.
- Some parts of the implementation may be imperfect; users should identify and optimize them as needed.
- It is highly recommended that developers use this for learning purposes and perform deep customizations and refactoring based on their own business needs (the `pinocchio` approach is recommended).

## Directory Structure

- `src/lib.rs`: Anchor contract entry point, containing the main `zooey` function for handling multi-pool arbitrage and balance management.
- `src/dex/`: Adapter layer for various DEX pool models (e.g., CLMM, CPMM, DLMM).
- `src/swap/`: Specific `swap` execution logic and aggregated arbitrage swap functionalities.
- `src/utils/`: Common utilities, math libraries, and calculation functions.
- `src/comparison.rs`: Core logic for arbitrage path comparison and calculation.
- `src/optimalamt.rs`: Algorithm for calculating the optimal arbitrage amount.
- ... Other modules.

## Getting Started

> **Note**: _This early-stage code is not recommended for direct production deployment. Use for local testing or educational purposes only._

```bash
# Anchor development environment is required
anchor build

# This project provides no security guarantees for any on-chain operations
```

## Suggestions

- It's recommended to `fork` this project, then optimize it by combining the design principles of `pinocchio` with your own requirements.
- Always conduct a code review and gain a deep understanding of the business logic.

---

For feedback or suggestions, please create a PR or an Issue.

<br>
---

<a name="zh-cn"></a>

# solana-onchain-program

> **⚠️ 本项目为作者初期编写，代码结构未作深度性能优化**
> **建议开发者基于自身理解重构，推荐参考 `pinocchio` 设计思路优化。**

---

## 项目简介

`solana-onchain-program` 是一个基于 Solana-Anchor 框架实现的多 DEX 智能套利（Onchain Arbitrage）合约。它能够扫描并利用主流 DEX 池模式（如 CLMM, CPMM, DAMM, DLMM, PUMP, RAYDIUM, WHIRLPOOL 等）进行套利。

### 核心功能
- **多池模式支持**: CLMM、DLMM、CPMM、DAMMV2、PUMP、Raydium、Whirlpool。
- **全局路径评估**: 识别提交的多代币、多池子参数，评估全局套利机会。
- **智能路径发现**: 自动寻找最佳的2跳（2-hop）和3跳（3-hop）套利路径，并计算最优交易量。
- **高兼容性**: 支持主网所有常见 DEX，无需为特定协议硬编码。
- **易于扩展**: 代码结构清晰，方便复用与重构。

### 特殊说明
- 本项目主要用于研究和二次开发。
- 部分代码实现可能不完善，需要使用者自行甄别和优化。
- 强烈建议开发者以学习为目的，并根据自身业务需求进行重构（推荐采用 `pinocchio` 方案）。

## 目录结构

- `src/lib.rs`：Anchor 合约入口，包含主函数 `zooey`，负责处理多池套利和余额管理。
- `src/dex/`：各类 DEX 池模型的适配层（如 CLMM, CPMM, DLMM 等）。
- `src/swap/`：具体的 `swap` 执行逻辑及聚合套利交换功能。
- `src/utils/`：通用工具、数学库和计算函数。
- `src/comparison.rs`：核心的套利路径比较和计算逻辑。
- `src/optimalamt.rs`：用于计算最优套利金额的算法。
- ... 其他各模块功能。

## 快速开始

```bash
# 需要 Anchor 开发环境
anchor build

# 部署和运行前，请务必参阅理解Anchor项目
# 本项目不对任何主网上链操作的安全性提供保证
```

## 建议

- 推荐 `fork` 本项目后，结合 `pinocchio` 的设计思想和自身需求进行优化重构。
- 务必进行代码审查（Code Review）并深度理解其业务逻辑。

---

如有任何反馈或建议，欢迎提交 PR 或 Issue。

---


