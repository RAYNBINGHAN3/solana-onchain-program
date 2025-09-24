use anchor_lang::prelude::*;
use crate::dex::cpmm::cpmm_program_id;
use crate::dex::dlmm::dlmm_program_id;
use crate::dex::dammv2::dammv2_program_id;
use crate::dex::pump::pump_program_id;
use crate::dex::raydium::raydium_program_id;
use crate::dex::clmm::clmm_program_id;
use crate::dex::whirlpool::whirlpool_program_id;
use anchor_spl::{
    token::{Token},
    token_2022::{self as token_2022_program},
    token_interface::spl_token_2022::{
        self,
        extension::{
            transfer_fee::{TransferFeeConfig},
            BaseStateWithExtensions,
            StateWithExtensions,
        }
    }
};

use crate::constant::{
    CPMM_ACCOUNT_COUNT, DLMM_ACCOUNT_COUNT, DAMMV2_ACCOUNT_COUNT, 
    PUMP_ACCOUNT_COUNT, RAYDIUM_ACCOUNT_COUNT, CLMM_ACCOUNT_COUNT, 
    WHIRLPOOL_ACCOUNT_COUNT
};


#[derive(Debug, Clone)]
pub struct TokenPoolGroup {
    pub token_mint_index: usize,
    pub token_program_index: usize,
    pub mint_token_account_index: usize,
    pub pools: Vec<PoolData>,
    pub is_3hop: bool,
    pub token2_mint_index: Option<usize>,
    pub token2_program_index: Option<usize>,
    pub token2_mint_token_account_index: Option<usize>,
}


#[derive(PartialEq, Eq, Debug, Clone, Copy)]
pub enum PoolType {
    CPMM,
    DLMM,
    DAMMV2,
    PUMP,
    RAYDIUM,
    CLMM,
    WHIRLPOOL
}


#[derive(Debug, Clone)]
pub enum PoolData {
    CPMM {
        program_id_index: usize,
        pool_index: usize,
        authority_index: usize,
        config_index: usize,
        token0_vault_index: usize,
        token1_vault_index: usize,
        observation_state_index: usize,
    },
    DLMM {
        program_id_index: usize,
        event_authority_index: usize,
        oracle_index: usize,
        pool_index: usize,
        reserve_x_index: usize,
        reserve_y_index: usize,
        bin_array_minus_1_index: usize,
        bin_array_0_index: usize,
        bin_array_1_index: usize,
    },
    DAMMV2 {
        program_id_index: usize,
        event_authority_index: usize,
        pool_authority_index: usize,
        pool_index: usize,
        token_a_vault_index: usize,
        token_b_vault_index: usize
    },
    PUMP {
        program_id_index: usize,
        pool_index: usize,
        global_config_index: usize,
        event_authority_index: usize,
        coin_creator_vault_ata_index: usize,
        coin_creator_vault_authority_index: usize,
        pump_fee_wallet_index: usize,
        pump_fee_wallet_ata_index: usize,
        global_vol_accumulator_index: usize,
        user_vol_accumulator_index: usize,
        system_program_index: usize,
        associated_token_program_index: usize,
        base_vault_index: usize,
        quote_vault_index: usize,
        fee_config_index: usize,
        fee_program_index: usize,
    },
    RAYDIUM {
        program_id_index: usize,
        pool_index: usize,
        event_authority_index: usize,
        base_vault_index: usize,
        quote_vault_index: usize,
    },
    CLMM {
        program_id_index: usize,
        pool_index: usize,
        amm_config_index: usize,
        observation_key_index: usize,
        bitmap_extension_index: usize,
        token_vault_0_index: usize,
        token_vault_1_index: usize,
        tick_array_minus_1_index: usize,
        tick_array_0_index: usize,
        tick_array_1_index: usize,
    },
    WHIRLPOOL {
        program_id_index: usize,
        pool_index: usize,
        oracle_index: usize,
        vault_a_index: usize,
        vault_b_index: usize,
        tick_array_0_index: usize,
        tick_array_1_index: usize,
        tick_array_2_index: usize,
    },
}

/// 获取转账手续费token2022 mint
pub fn get_transfer_fee(mint_info: &AccountInfo, pre_fee_amount: u64) -> Result<u64> {
    if *mint_info.owner == Token::id() {
        return Ok(0);
    }
    let mint_data = mint_info.try_borrow_data()?;
    let mint = StateWithExtensions::<spl_token_2022::state::Mint>::unpack(&mint_data)?;

    let fee = if let Ok(transfer_fee_config) = mint.get_extension::<TransferFeeConfig>() {
        transfer_fee_config
            .calculate_epoch_fee(Clock::get()?.epoch, pre_fee_amount)
            .unwrap()
    } else {
        0
    };
    Ok(fee)
}

/// 判断token program是否为Token2022
pub fn is_token_2022(token_program_id: Pubkey) -> bool {
    token_program_id == token_2022_program::ID
}



/// 获取池类型
pub fn get_pool_type(pool_program_id: Pubkey) -> Result<Option<PoolType>> {
    if pool_program_id == cpmm_program_id::ID {
        Ok(Some(PoolType::CPMM))
    } else if pool_program_id == dlmm_program_id::ID {
        Ok(Some(PoolType::DLMM))
    } else if pool_program_id == dammv2_program_id::ID {
        Ok(Some(PoolType::DAMMV2))
    } else if pool_program_id == pump_program_id::ID {
        Ok(Some(PoolType::PUMP))
    } else if pool_program_id == raydium_program_id::ID {
        Ok(Some(PoolType::RAYDIUM))
    } else if pool_program_id == clmm_program_id::ID {
        Ok(Some(PoolType::CLMM))
    } else if pool_program_id == whirlpool_program_id::ID {
        Ok(Some(PoolType::WHIRLPOOL))
    } else {
        Ok(None)
    }
}


/// 解析 remaining_accounts，构建 TokenPoolGroup 列表
#[inline]
pub fn parse_token_groups<'info>(remaining_accounts: &'info [AccountInfo<'info>]) -> Result<Vec<TokenPoolGroup>> {
    let mut current_index = 0;
    let mut token_groups = Vec::with_capacity(3);

    while current_index + 3 < remaining_accounts.len() {
        let token_mint_index = current_index;
        let token_program_index = current_index + 1;
        let mint_token_account_index = current_index + 2;

        // 判断是否为3hop（通过占位的memo_program账户）
        let is_3hop = if remaining_accounts[current_index + 3].key() == remaining_accounts[current_index + 2].key() { true } else { false };
        let (token2_mint_index, token2_program_index, token2_mint_token_account_index) = if is_3hop {
            (Some(current_index + 4), Some(current_index + 5), Some(current_index + 6))
        } else {
            (None, None, None)
        };

        let mut pools = Vec::with_capacity(6);
        current_index += if is_3hop { 7 } else { 3 };

        while current_index < remaining_accounts.len() {
            let pool_program_id = &remaining_accounts[current_index];

            match get_pool_type(pool_program_id.key()) {
                Ok(pool_type) => match pool_type {
                    Some(PoolType::CPMM) => {
                        if current_index + CPMM_ACCOUNT_COUNT <= remaining_accounts.len() {
                            pools.push(PoolData::CPMM {
                                program_id_index: current_index,
                                authority_index: current_index + 1,
                                config_index: current_index + 2,
                                observation_state_index: current_index + 3,
                                pool_index: current_index + 4,
                                token0_vault_index: current_index + 5,
                                token1_vault_index: current_index + 6,
                            });
                            current_index += CPMM_ACCOUNT_COUNT;
                        } else {
                            break;
                        }
                    }
                    Some(PoolType::DLMM) => {
                        if current_index + DLMM_ACCOUNT_COUNT <= remaining_accounts.len() {
                            pools.push(PoolData::DLMM {
                                program_id_index: current_index,
                                event_authority_index: current_index + 1,
                                oracle_index: current_index + 2,
                                pool_index: current_index + 3,
                                reserve_x_index: current_index + 4,
                                reserve_y_index: current_index + 5,
                                bin_array_minus_1_index: current_index + 6,
                                bin_array_0_index: current_index + 7,
                                bin_array_1_index: current_index + 8,
                            });
                            current_index += DLMM_ACCOUNT_COUNT;
                        } else {
                            break;
                        }
                    }
                    Some(PoolType::DAMMV2) => {
                        if current_index + DAMMV2_ACCOUNT_COUNT <= remaining_accounts.len() {
                            pools.push(PoolData::DAMMV2 {
                                program_id_index: current_index,
                                event_authority_index: current_index + 1,
                                pool_authority_index: current_index + 2,
                                pool_index: current_index + 3,
                                token_a_vault_index: current_index + 4,
                                token_b_vault_index: current_index + 5,
                            });
                            current_index += DAMMV2_ACCOUNT_COUNT;
                        } else {
                            break;
                        }
                    }
                    Some(PoolType::PUMP) => {
                        if current_index + PUMP_ACCOUNT_COUNT <= remaining_accounts.len() {
                            pools.push(PoolData::PUMP {
                                program_id_index: current_index,
                                pool_index: current_index + 1,
                                global_config_index: current_index + 2,
                                event_authority_index: current_index + 3,
                                coin_creator_vault_ata_index: current_index + 4,
                                coin_creator_vault_authority_index: current_index + 5,
                                pump_fee_wallet_index: current_index + 6,
                                pump_fee_wallet_ata_index: current_index + 7,
                                global_vol_accumulator_index: current_index + 8,
                                user_vol_accumulator_index: current_index + 9,
                                system_program_index: current_index + 10,
                                associated_token_program_index: current_index + 11,
                                base_vault_index: current_index + 12,
                                quote_vault_index: current_index + 13,
                                fee_config_index: current_index + 14,
                                fee_program_index: current_index + 15,
                            });
                            current_index += PUMP_ACCOUNT_COUNT;
                        } else {
                            break;
                        }
                    }
                    Some(PoolType::RAYDIUM) => {
                        if current_index + RAYDIUM_ACCOUNT_COUNT <= remaining_accounts.len() {
                            pools.push(PoolData::RAYDIUM {
                                program_id_index: current_index,
                                event_authority_index: current_index + 1,
                                pool_index: current_index + 2,
                                base_vault_index: current_index + 3,
                                quote_vault_index: current_index + 4,
                            });
                            current_index += RAYDIUM_ACCOUNT_COUNT;
                        } else {
                            break;
                        }
                    }
                    Some(PoolType::CLMM) => {
                        if current_index + CLMM_ACCOUNT_COUNT <= remaining_accounts.len() {
                            pools.push(PoolData::CLMM {
                                program_id_index: current_index,
                                pool_index: current_index + 1,
                                amm_config_index: current_index + 2,
                                observation_key_index: current_index + 3,
                                bitmap_extension_index: current_index + 4,
                                token_vault_0_index: current_index + 5,
                                token_vault_1_index: current_index + 6,
                                tick_array_minus_1_index: current_index + 7,
                                tick_array_0_index: current_index + 8,
                                tick_array_1_index: current_index + 9,
                            });
                            current_index += CLMM_ACCOUNT_COUNT;
                        } else {
                            break;
                        }
                    }
                    Some(PoolType::WHIRLPOOL) => {
                        if current_index + WHIRLPOOL_ACCOUNT_COUNT <= remaining_accounts.len() {
                            pools.push(PoolData::WHIRLPOOL {
                                program_id_index: current_index,
                                pool_index: current_index + 1,
                                oracle_index: current_index + 2,
                                vault_a_index: current_index + 3,
                                vault_b_index: current_index + 4,
                                tick_array_0_index: current_index + 5,
                                tick_array_1_index: current_index + 6,
                                tick_array_2_index: current_index + 7,
                            });
                            current_index += WHIRLPOOL_ACCOUNT_COUNT;
                        } else {
                            break;
                        }
                    }
                    None => {
                        break;
                    }
                },
                Err(_) => {
                    break;
                }
            }
        }

        if !pools.is_empty() {
            token_groups.push(TokenPoolGroup {
                token_mint_index,
                token_program_index,
                mint_token_account_index,
                pools,
                is_3hop,
                token2_mint_index,
                token2_program_index,
                token2_mint_token_account_index,
            });
        }
    }

    Ok(token_groups)
}
