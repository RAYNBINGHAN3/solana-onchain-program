use anchor_lang::prelude::*;
use crate::dex::cpmm::cpmm_program_id;
use crate::dex::dlmm::dlmm_program_id;
use crate::dex::dammv2::dammv2_program_id;
use crate::dex::pump::pump_program_id;

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
    }
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
    } else {
        Ok(None)
    }
}

 