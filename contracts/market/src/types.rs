//! Market contract types.
//!
//! Types shared across contracts (enums, `Fighter`, `Bet`, `ClaimReceipt`,
//! `ProtocolConfig`) are re-exported from `shared::types` so every contract
//! encodes them with identical XDR. Only types that exist solely in the Market
//! contract are defined here:
//!
//! - `Market`: the full per-instance market state. It is a superset of
//!   `shared::types::Market` (adds `resolved_at`, `dispute_window_sec`,
//!   `treasury`, `bet_token`, and stores the outcome as `Option<Outcome>`),
//!   and is persisted only by the Market contract itself.
//! - `SettledOutcome`, `MarketResolved`, `WinningsClaimed`: market-local
//!   settlement / event payloads.

use soroban_sdk::{contracttype, Address, Bytes};

pub use shared::types::{
    Bet, BetSide, ClaimReceipt, Fighter, MarketStatus, Outcome, ProtocolConfig,
};

/// Post-resolution outcome stored in Market. Pending until the market resolves.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub enum SettledOutcome {
    Pending,
    FighterA,
    FighterB,
    Draw,
    NoContest,
}

/// Full market state held by a deployed Market contract instance (market-only).
#[contracttype]
#[derive(Clone, Debug)]
pub struct Market {
    pub market_id: Bytes,
    pub fighter_a: Fighter,
    pub fighter_b: Fighter,
    pub scheduled_at: u64,
    pub betting_ends_at: u64,
    pub created_at: u64,
    pub created_by: Address,
    pub status: MarketStatus,
    pub pool_a: i128,
    pub pool_b: i128,
    pub total_pool: i128,
    pub protocol_fee_bp: u32,
    pub oracle_address: Address,
    pub outcome: Option<Outcome>,
    pub fee_collector_address: Address,
    pub resolved_at: u64,
    pub dispute_window_sec: u64,
    pub treasury: Address,
    pub bet_token: Address,
}

#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct MarketResolved {
    pub market_id: Bytes,
    pub outcome: Outcome,
    pub resolved_at: u64,
}

#[contracttype]
#[derive(Clone, Debug)]
pub struct WinningsClaimed {
    pub bet_id: Bytes,
    pub bettor: Address,
    pub payout: i128,
    pub fee_paid: i128,
    pub claimed_at: u64,
}
