
// pub const WSOL_MINT: &str = "So11111111111111111111111111111111111111112";
pub const BASE_BPS: u64 = 1_000_000;

/// 每种池类型需要的账户数量（包括程序ID和池账户）
pub const CPMM_ACCOUNT_COUNT: usize = 7; // program_id + authority + pool + config + token0_vault + token1_vault + observation_state
pub const DLMM_ACCOUNT_COUNT: usize = 9; // program_id + event_authority + oracle + pool + reserve_x + reserve_y + bin_array(-1) + bin_array(0) + bin_array(1) 
pub const DAMMV2_ACCOUNT_COUNT: usize = 6; // program_id + event_authority + pool_authority + pool + token_a_vault + token_b_vault
pub const PUMP_ACCOUNT_COUNT: usize = 16; // program_id + pool + global_config + event_authority + coin_creator_vault_ata + coin_creator_vault_authority + pump_fee_wallet + pump_fee_wallet_ata + global_vol_accumulator + user_vol_accumulator + system_program + associated_token_program + base_vault + quote_vault + fee_config + fee_program
pub const RAYDIUM_ACCOUNT_COUNT: usize = 5; // program_id + event_authority + pool + base_vault + quote_vault
pub const CLMM_ACCOUNT_COUNT: usize = 10; // program_id + pool + amm_config + observation_key + token_vault_0 + token_vault_1 + tick_array_minus_1 + tick_array_0 + tick_array_1 + bitmap_extension
pub const WHIRLPOOL_ACCOUNT_COUNT: usize = 8; // program_id + pool + oracle + vault_a + vault_b + tick_array_0 + tick_array_1 + tick_array_2


pub const WSOL_MINT: [u8; 32] = [
    6, 155, 136, 87, 254, 171, 129, 132, 251, 104, 127, 99, 70, 24, 192, 53,
    218, 196, 57, 220, 26, 235, 59, 85, 9, 160, 87, 149, 45, 129, 81, 81
];



