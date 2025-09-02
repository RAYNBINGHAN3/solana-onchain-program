use anchor_lang::prelude::*;
use crate::dex::cpmm::cpmm_program_id;
use crate::dex::dlmm::dlmm_program_id;
use crate::dex::dammv2::dammv2_program_id;
use crate::dex::pump::pump_program_id;
use crate::dex::raydium::raydium_program_id;
use crate::dex::clmm::clmm_program_id;
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


#[derive(Debug, Clone)]
pub struct TokenPoolGroup {
    pub token_mint_index: usize,
    pub token_program_index: usize,
    pub mint_token_account_index: usize,
    pub pools: Vec<PoolData>,
}


#[derive(PartialEq, Eq, Debug, Clone, Copy)]
pub enum PoolType {
    CPMM,
    DLMM,
    DAMMV2,
    PUMP,
    RAYDIUM,
    CLMM,
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
    } else {
        Ok(None)
    }
}