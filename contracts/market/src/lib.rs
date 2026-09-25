#![no_std]

pub mod types;

use shared::{events, market_id_to_u64};
use soroban_sdk::{
    contract, contractimpl, contracttype, Address, Bytes, Env, IntoVal, String, Symbol, Vec,
};
use types::{Bet, BetSide, Fighter, Market, MarketStatus, Outcome, ProtocolConfig, SettledOutcome};

// ─── STORAGE KEYS ─────────────────────────────────────────────────────────────
// DataKey::MarketInfo     -> Market
// DataKey::Factory        -> Address  (MarketFactory contract address)
// DataKey::Bet(id)        -> Bet
// DataKey::BetsByAddr(a)  -> Vec<Bytes>  (all bet_ids for an address)
// DataKey::Claimed(id)    -> bool
// DataKey::DisputeRaised  -> bool
// DataKey::DisputeReason  -> Bytes
// "BET_COUNT"             -> u64

#[contracttype]
pub enum DataKey {
    MarketInfo,
    Factory,
    Bet(Bytes),
    BetsByAddr(Address),
    Claimed(Bytes),
    DisputeRaised,
    DisputeReason,
}

#[contract]
pub struct MarketContract;

#[contractimpl]
impl MarketContract {
    fn read_market(env: &Env) -> Market {
        env.storage()
            .persistent()
            .get(&DataKey::MarketInfo)
            .expect("market not initialized")
    }

    fn write_market(env: &Env, market: &Market) {
        env.storage().persistent().set(&DataKey::MarketInfo, market);
    }

    /// Called by MarketFactory immediately after contract deployment.
    /// Initializes a new boxing prediction market.
    ///
    /// Called by `MarketFactory` immediately after contract deployment.
    /// Stores all market metadata and initializes pool values to 0, with status set to `Open`.
    ///
    /// # Arguments
    ///
    /// * `env` - The Soroban execution environment.
    /// * `market_id` - Unique identifier for this market (32-byte hash).
    /// * `fighter_a` - Metadata for the first fighter.
    /// * `fighter_b` - Metadata for the second fighter.
    /// * `scheduled_at` - Unix timestamp (seconds) of the scheduled fight time.
    /// * `betting_ends_at` - Unix timestamp after which no new bets are accepted.
    /// * `oracle` - Address authorized to lock and resolve this market.
    /// * `factory` - Address of the deploying `MarketFactory` contract.
    /// * `protocol_fee_bp` - Protocol fee in basis points (e.g. `200` = 2%).
    /// * `fee_collector` - Address that receives the protocol fee on payouts.
    /// * `dispute_window_sec` - Duration in seconds during which disputes can be raised after resolution.
    /// * `treasury` - Address of the `Treasury` contract used to escrow bet funds.
    /// * `bet_token` - Address of the token contract accepted for bets on this market.
    ///
    /// # Panics
    ///
    /// Panics if:
    /// - The market has already been initialized.
    /// - `betting_ends_at` (lock time) is after `scheduled_at` (end time).
    pub fn initialize(
        env: Env,
        market_id: Bytes,
        fighter_a: Fighter,
        fighter_b: Fighter,
        scheduled_at: u64,
        betting_ends_at: u64,
        oracle: Address,
        factory: Address,
        protocol_fee_bp: u32,
        fee_collector: Address,
        dispute_window_sec: u64,
        treasury: Address,
        bet_token: Address,
    ) {
        if env.storage().persistent().has(&DataKey::MarketInfo) {
            panic!("already initialized");
        }
        if betting_ends_at > scheduled_at {
            panic!("lock time must be at or before end time");
        }
        let market = Market {
            market_id: market_id.clone(),
            fighter_a,
            fighter_b,
            scheduled_at,
            betting_ends_at,
            created_at: env.ledger().timestamp(),
            created_by: factory.clone(),
            status: MarketStatus::Open,
            pool_a: 0,
            pool_b: 0,
            total_pool: 0,
            protocol_fee_bp,
            oracle_address: oracle,
            outcome: SettledOutcome::Pending,
            fee_collector_address: fee_collector,
            resolved_at: 0,
            dispute_window_sec,
            treasury,
            bet_token,
        };
        env.storage().persistent().set(&DataKey::MarketInfo, &market);
        env.storage().persistent().set(&DataKey::Factory, &factory);

        // Emit market_created event with contract address and market info
        // Topics: (Symbol("market_created"), market_id)
        // Data: Market struct (matches MarketInfo field-for-field)
        env.events().publish(
            (Symbol::new(&env, "market_created"), market_id),
            market,
        );
    }

    /// Places a bet on a fighter in this market.
    ///
    /// Transfers XLM from `bettor` to this contract (escrow), records the bet,
    /// updates the relevant pool, and emits a `BetPlaced` event.
    ///
    /// # Arguments
    ///
    /// * `env` - The Soroban execution environment.
    /// * `bettor` - Address of the user placing the bet. Must authorize this call.
    /// * `side` - Which fighter to bet on (`BetSide::FighterA` or `BetSide::FighterB`).
    /// * `amount` - Bet amount in stroops. Must satisfy `min_bet_amount ≤ amount ≤ max_bet_amount`.
    ///
    /// # Returns
    ///
    /// Returns the unique `bet_id` (`Bytes`) assigned to this bet.
    ///
    /// # Panics
    ///
    /// Panics if:
    /// - The market status is not `Open`.
    /// - The current ledger time is at or after `betting_ends_at`.
    /// - `amount` is below the configured `min_bet_amount`.
    /// - `amount` is above the configured `max_bet_amount`.
    /// - `bettor` has not authorized the call.
    pub fn place_bet(
        env: Env,
        bettor: Address,
        side: BetSide,
        amount: i128,
    ) -> Bytes {
        bettor.require_auth();

        let mut market = Self::read_market(&env);

        if market.status != MarketStatus::Open {
            panic!("market not open");
        }
        if env.ledger().timestamp() >= market.betting_ends_at {
            panic!("betting period has ended");
        }

        let factory: Address = env.storage().persistent()
            .get(&DataKey::Factory)
            .expect("factory not set");
        let config: ProtocolConfig = env.invoke_contract(
            &factory,
            &Symbol::new(&env, "get_config"),
            soroban_sdk::vec![&env],
        );

        // Prevent dust bets that consume on-chain storage without contributing
        // meaningful opposing liquidity. The configured min_bet_amount is checked
        // before any escrow transfer or state mutation.
        if amount < config.min_bet_amount {
            panic!("below minimum bet");
        }
        if amount > config.max_bet_amount {
            panic!("above maximum bet");
        }

        if amount <= 0 {
            panic!("amount must be positive");
        }

        // Escrow the bet amount via the Treasury. This is a cross-contract call
        // that aborts the whole transaction on failure, so the bet is only ever
        // recorded below once the deposit has actually succeeded.
        env.invoke_contract::<()>(
            &market.treasury,
            &Symbol::new(&env, "deposit"),
            soroban_sdk::vec![
                &env,
                env.current_contract_address().into_val(&env),
                market.market_id.clone().into_val(&env),
                bettor.clone().into_val(&env),
                amount.into_val(&env),
            ],
        );

        match side {
            BetSide::FighterA => market.pool_a = market.pool_a.checked_add(amount).expect("pool_a overflow"),
            BetSide::FighterB => market.pool_b = market.pool_b.checked_add(amount).expect("pool_b overflow"),
        }
        market.total_pool = market.total_pool.checked_add(amount).expect("total_pool overflow");

        let bet_count: u64 = env.storage().persistent()
            .get(&Symbol::new(&env, "BET_COUNT"))
            .unwrap_or(0u64);
        let new_count = bet_count + 1;
        env.storage().persistent().set(&Symbol::new(&env, "BET_COUNT"), &new_count);

        let mut id_bytes = [0u8; 32];
        id_bytes[..8].copy_from_slice(&new_count.to_be_bytes());
        let bet_id = Bytes::from_array(&env, &id_bytes);

        let placed_at = env.ledger().timestamp();
        let bet = Bet {
            bet_id: bet_id.clone(),
            market_id: market.market_id.clone(),
            bettor: bettor.clone(),
            side: side.clone(),
            amount,
            placed_at,
            claimed: false,
        };
        env.storage().persistent().set(&DataKey::Bet(bet_id.clone()), &bet);

        let mut bets: Vec<Bytes> = env
            .storage()
            .persistent()
            .get(&DataKey::BetsByAddr(bettor.clone()))
            .unwrap_or(Vec::new(&env));
        bets.push_back(bet_id.clone());
        env.storage()
            .persistent()
            .set(&DataKey::BetsByAddr(bettor.clone()), &bets);

        Self::write_market(&env, &market);

        let market_id_u64 = market_id_to_u64(&market.market_id);
        events::emit_bet_placed(
            &env,
            market_id_u64,
            shared::types::BetRecord {
                bet_id: bet_id.clone(),
                bettor,
                market_id: market_id_u64,
                side: side.into(),
                amount,
                placed_at,
                claimed: false,
            },
        );

        bet_id
    }

    /// Transitions market status from Open to Locked.
    /// Admin-only. Cancels a market (e.g. fight postponed).
    /// require_auth() is the first call. Verifies caller is the factory admin.
    /// Valid only when status is Open or Locked. Emits MarketCancelled event.
    pub fn cancel_market(env: Env, admin: Address) {
        admin.require_auth();

        let factory: Address = env
            .storage()
            .persistent()
            .get(&DataKey::Factory)
            .expect("factory not set");
        let config: ProtocolConfig = env.invoke_contract(
            &factory,
            &Symbol::new(&env, "get_config"),
            soroban_sdk::vec![&env],
        );
        if config.admin != admin {
            panic!("not factory admin");
        }

        let mut market = Self::read_market(&env);
        match market.status {
            MarketStatus::Open | MarketStatus::Locked => {}
            _ => panic!("cannot cancel: market already resolved or cancelled"),
        }

        market.status = MarketStatus::Cancelled;
        Self::write_market(&env, &market);

        events::emit_market_cancelled(
            &env,
            market_id_to_u64(&market.market_id),
            String::from_str(&env, "cancelled_by_admin"),
        );
    }

    /// Transitions the market status from `Open` to `Locked`.
    ///
    /// After locking, no new bets are accepted. Can be called by the oracle address
    /// at any time, or by anyone once `betting_ends_at` has passed.
    /// Emits a `MarketLocked` event.
    ///
    /// # Arguments
    ///
    /// * `env` - The Soroban execution environment.
    /// * `oracle` - Address of the oracle or any caller after the betting period ends.
    ///
    /// # Panics
    ///
    /// Panics if the market status is not `Open`, or if `oracle` is not the
    /// authorized oracle address and the betting period has not yet ended.
    pub fn lock_market(env: Env, oracle: Address) {
        let mut market = Self::read_market(&env);

        if market.status != MarketStatus::Open {
            panic!("market already locked");
        }

        let now = env.ledger().timestamp();
        if now < market.betting_ends_at {
            // Early lock: only the market's oracle may lock before lock_time passes.
            oracle.require_auth();
            if oracle != market.oracle_address {
                panic!("not authorized oracle");
            }
        }
        // Once lock_time has passed, locking is permissionless — no auth required.

        market.status = MarketStatus::Locked;
        Self::write_market(&env, &market);

        events::emit_market_locked(&env, market_id_to_u64(&market.market_id), now);
    }

    /// Called by oracle after fight concludes.
    /// Draw outcome sets status to Cancelled so both sides can claim full refunds.
    /// Records the fight outcome and resolves the market.
    ///
    /// Called by the oracle after the fight concludes. Sets the outcome and
    /// transitions status to `Resolved`. If `outcome` is `NoContest`, status is
    /// set to `Cancelled` instead, enabling full refunds. Emits a `MarketResolved` event.
    ///
    /// # Arguments
    ///
    /// * `env` - The Soroban execution environment.
    /// * `oracle` - Address of the authorized oracle. Must authorize this call.
    /// * `outcome` - The fight result (`FighterA`, `FighterB`, `Draw`, or `NoContest`).
    ///
    /// # Panics
    ///
    /// Panics if:
    /// - The caller is not the authorized oracle address.
    /// - The market status is not `Locked`.
    pub fn resolve_market(env: Env, oracle: Address, outcome: Outcome) {
        oracle.require_auth();

        let mut market: Market = env.storage().persistent()
            .get(&DataKey::MarketInfo)
            .expect("market not initialized");

        if market.status != MarketStatus::Locked {
            panic!("market not locked");
        }

        if market.oracle_address != oracle {
            panic!("not authorized oracle");
        }

        // Set resolved_at timestamp for dispute window enforcement
        market.resolved_at = env.ledger().timestamp();

        // Draw reuses the Cancelled path so both sides receive full refunds with no fee.
        market.status = match outcome {
            Outcome::NoContest | Outcome::Draw => MarketStatus::Cancelled,
            _ => MarketStatus::Resolved,
        };
        market.outcome = outcome.clone().into();
        let resolution_time = env.ledger().timestamp();
        env.storage().persistent().set(&DataKey::MarketInfo, &market);

        events::emit_market_resolved(
            &env,
            market_id_to_u64(&market.market_id),
            outcome.into(),
            resolution_time,
        );
    }

    /// Allows a winning bettor to claim their proportional share of the pool.
    /// Payout = bettor_stake / winning_pool * net_pool (fee already deducted).
    /// Pays out winnings to a bettor whose bet matched the fight outcome.
    ///
    /// Payout formula: `(bettor_stake / winning_pool) * total_pool * (1 - fee_bp / 10_000)`.
    /// The protocol fee portion is transferred to `fee_collector`.
    /// The `CLAIMED` flag is set before any transfer to guard against re-entrancy.
    /// Emits a `WinningsClaimed` event.
    ///
    /// # Arguments
    ///
    /// * `env` - The Soroban execution environment.
    /// * `bettor` - Address of the bettor claiming winnings. Must authorize this call.
    /// * `bet_id` - Unique identifier of the bet to claim.
    ///
    /// # Returns
    ///
    /// Returns the payout amount transferred to `bettor`, in stroops.
    ///
    /// # Panics
    ///
    /// Panics if:
    /// - `bettor` has not authorized the call.
    /// - `bet_id` does not exist.
    /// - `bettor` is not the owner of the bet.
    /// - The market status is not `Resolved`.
    /// - The bet's side does not match the winning outcome.
    /// - The bet has already been claimed.
    pub fn claim_winnings(env: Env, bettor: Address, bet_id: Bytes) -> i128 {
        bettor.require_auth();

        let bet: Bet = env.storage().persistent()
            .get(&DataKey::Bet(bet_id.clone()))
            .expect("bet not found");
        if bet.bettor != bettor {
            panic!("not your bet");
        }

        let market: Market = env.storage().persistent()
            .get(&DataKey::MarketInfo)
            .expect("market not initialized");
        if market.status != MarketStatus::Resolved {
            panic!("market not resolved");
        }

        let outcome = market.outcome.outcome().expect("no outcome set");
        let is_winner = match (&bet.side, &outcome) {
            (BetSide::FighterA, Outcome::FighterA) => true,
            (BetSide::FighterB, Outcome::FighterB) => true,
            _ => false,
        };
        if !is_winner {
            panic!("bet did not win");
        }

        let already_claimed: bool = env.storage().persistent()
            .get(&DataKey::Claimed(bet_id.clone()))
            .unwrap_or(false);
        if already_claimed {
            panic!("already claimed");
        }

        let winning_pool = match outcome {
            Outcome::FighterA => market.pool_a,
            Outcome::FighterB => market.pool_b,
            _ => market.pool_a.checked_add(market.pool_b).expect("pool sum overflow"),
        };

        let payout = if winning_pool > 0 {
            let fee_amount = shared::types::calculate_fee(market.total_pool, market.protocol_fee_bp);
            let net_pool = market.total_pool.checked_sub(fee_amount).expect("net pool underflow");
            bet.amount
                .checked_mul(net_pool)
                .expect("payout overflow")
                .checked_div(winning_pool)
                .expect("payout div zero")
        } else {
            0
        };

        // Mark claimed BEFORE any transfer (re-entrancy guard).
        env.storage().persistent().set(&DataKey::Claimed(bet_id.clone()), &true);

        let receipt = shared::types::ClaimReceipt {
            bet_id: bet_id.clone(),
            bettor: bettor.clone(),
            payout,
            claimed_at: env.ledger().timestamp(),
        };
        events::emit_winnings_claimed(&env, market_id_to_u64(&market.market_id), receipt);

        payout
    }

    /// Issues a full refund when market is Cancelled (includes Draw and NoContest outcomes).
    /// No protocol fee deducted on refunds.
    ///
    /// Applicable when market status is `Cancelled` or outcome is `NoContest`.
    /// No protocol fee is deducted on refunds. The `CLAIMED` flag is set before
    /// any transfer to guard against re-entrancy. Emits a `RefundClaimed` event.
    ///
    /// # Arguments
    ///
    /// * `env` - The Soroban execution environment.
    /// * `bettor` - Address of the bettor claiming the refund. Must authorize this call.
    /// * `bet_id` - Unique identifier of the bet to refund.
    ///
    /// # Returns
    ///
    /// Returns the refund amount (equal to the original `bet.amount`), in stroops.
    ///
    /// # Panics
    ///
    /// Panics if:
    /// - `bettor` has not authorized the call.
    /// - `bet_id` does not exist.
    /// - `bettor` is not the owner of the bet.
    /// - The market status is not `Cancelled` and outcome is not `NoContest`.
    /// - The bet has already been claimed.
    /// Full refund for a bet when market is Cancelled. No protocol fee.
    pub fn claim_refund(env: Env, bettor: Address, bet_id: Bytes) -> i128 {
        bettor.require_auth();

        let bet: Bet = env
            .storage()
            .persistent()
            .get(&DataKey::Bet(bet_id.clone()))
            .expect("bet not found");
        if bet.bettor != bettor {
            panic!("not your bet");
        }

        let market = Self::read_market(&env);
        // Check market is Cancelled or has NoContest outcome
        let is_eligible = match market.status {
            MarketStatus::Cancelled => true,
            MarketStatus::Resolved => {
                market.outcome == SettledOutcome::NoContest
            }
            _ => false,
        };
        if !is_eligible {
            panic!("market not eligible for refund");
        }

        let already_claimed: bool = env
            .storage()
            .persistent()
            .get(&DataKey::Claimed(bet_id.clone()))
            .unwrap_or(false);
        if already_claimed {
            panic!("already claimed");
        }

        // Mark claimed BEFORE any transfer (re-entrancy guard)
        env.storage()
            .persistent()
            .set(&DataKey::Claimed(bet_id.clone()), &true);

        events::emit_refund_claimed(
            &env,
            market_id_to_u64(&market.market_id),
            bettor,
            bet_id,
            bet.amount,
        );

        bet.amount
    }


    /// Dispute resolution - allows bettors to challenge submitted market resolutions.
    ///
    /// Transitions status to `Disputed`, freezing all claim processing until an admin
    /// settles the dispute. Must be called within `dispute_window_sec` of `resolved_at`.
    /// Only one active dispute is allowed per market. Emits a `resolution_disputed` event.
    ///
    /// # Arguments
    ///
    /// * `env` - The Soroban execution environment.
    /// * `bettor` - Address of the bettor raising the dispute. Must authorize this call
    ///   and must have an existing bet in this market.
    /// * `reason` - Free-form bytes describing the reason for the dispute.
    ///
    /// # Panics
    ///
    /// Panics if:
    /// - `bettor` has not authorized the call.
    /// - `bettor` has no bet in this market.
    /// - The dispute window has elapsed since resolution.
    /// - A dispute is already active on this market.
    /// - The market status is not `Resolved`.
    pub fn dispute_resolution(env: Env, bettor: Address, reason: Bytes) {
        bettor.require_auth();

        let mut market = Self::read_market(&env);

        if market.status != MarketStatus::Resolved {
            panic!("market not resolved");
        }

        // Check if a dispute has already been raised
        let already_disputed: bool = env.storage().persistent()
            .get(&DataKey::DisputeRaised)
            .unwrap_or(false);
        if already_disputed {
            panic!("dispute already raised");
        }

        // Verify bettor has a bet in this market
        let bettor_bets: Vec<Bytes> = env.storage().persistent()
            .get(&DataKey::BetsByAddr(bettor.clone()))
            .unwrap_or(Vec::new(&env));
        if bettor_bets.is_empty() {
            panic!("bettor has no bets in this market");
        }

        // Check if within dispute window
        let current_time = env.ledger().timestamp();
        let dispute_deadline = market.resolved_at + market.dispute_window_sec;
        if current_time > dispute_deadline {
            panic!("dispute window has closed");
        }

        // Transition to Disputed status
        market.status = MarketStatus::Disputed;
        Self::write_market(&env, &market);

        // Cap reason length to prevent storage abuse (max 256 bytes)
        let max_reason_len = 256;
        if reason.len() > max_reason_len {
            panic!("dispute reason exceeds maximum length");
        }

        // Store dispute reason
        env.storage().persistent().set(&DataKey::DisputeRaised, &true);
        env.storage().persistent().set(&DataKey::DisputeReason, &reason);

        events::emit_resolution_disputed(
            &env,
            market_id_to_u64(&market.market_id),
            bettor,
            reason,
        );
    }

    /// Settles a disputed market with a final admin-override outcome.
    ///
    /// The override outcome may differ from the oracle's original outcome.
    /// Transitions status back to `Resolved`, re-opening claims with the new outcome.
    /// Emits a `DisputeResolved` event.
    ///
    /// # Arguments
    ///
    /// * `env` - The Soroban execution environment.
    /// * `admin` - Address of the protocol admin. Must authorize this call.
    /// * `override_outcome` - The admin-determined final outcome for the market.
    ///
    /// # Panics
    ///
    /// Panics if:
    /// - `admin` has not authorized the call or is not the configured admin.
    /// - The market status is not `Disputed`.
    pub fn resolve_dispute(env: Env, admin: Address, override_outcome: Outcome) {
        admin.require_auth();

        let factory: Address = env
            .storage()
            .persistent()
            .get(&DataKey::Factory)
            .expect("factory not set");
        let config: ProtocolConfig = env.invoke_contract(
            &factory,
            &Symbol::new(&env, "get_config"),
            soroban_sdk::vec![&env],
        );
        if config.admin != admin {
            panic!("not factory admin");
        }

        let mut market = Self::read_market(&env);
        if market.status != MarketStatus::Disputed {
            panic!("market not in disputed state");
        }

        market.outcome = override_outcome.clone().into();
        market.status = MarketStatus::Resolved;
        Self::write_market(&env, &market);

        events::emit_dispute_resolved(
            &env,
            market_id_to_u64(&market.market_id),
            override_outcome.into(),
        );
    }

    /// Finalizes the market resolution after dispute window expires or admin override.
    ///
    /// Supports two scenarios:
    /// 1. Permissionless finalization when market is Resolved and dispute window has elapsed
    /// 2. Admin-controlled finalization when market is Disputed (admin-only)
    ///
    /// After finalization, the `claim_winnings` function becomes available.
    /// Emits a `ResolutionFinalized` event.
    ///
    /// # Arguments
    ///
    /// * `env` - The Soroban execution environment.
    /// * `admin` - (Optional) Address of the protocol admin. Required only when market is Disputed.
    ///           If market is Resolved and window has elapsed, any caller can finalize.
    ///
    /// # Panics
    ///
    /// Panics if:
    /// - Market status is Resolved but dispute window has not elapsed yet.
    /// - Market status is Disputed but caller is not the admin.
    /// - Market is in any other status (Open, Locked, Cancelled).
    pub fn finalize_resolution(env: Env, admin: Option<Address>) {
        let mut market = Self::read_market(&env);

        match market.status {
            MarketStatus::Resolved => {
                let current_time = env.ledger().timestamp();
                let dispute_deadline = market.resolved_at + market.dispute_window_sec;
                if current_time <= dispute_deadline {
                    panic!("dispute window still open");
                }
                market.status = MarketStatus::Resolved;
                Self::write_market(&env, &market);
            }
            MarketStatus::Disputed => {
                if let Some(admin_addr) = admin {
                    admin_addr.require_auth();
                    let factory: Address = env
                        .storage()
                        .persistent()
                        .get(&DataKey::Factory)
                        .expect("factory not set");
                    let config: ProtocolConfig = env.invoke_contract(
                        &factory,
                        &Symbol::new(&env, "get_config"),
                        soroban_sdk::vec![&env],
                    );
                    if config.admin != admin_addr {
                        panic!("not factory admin");
                    }
                } else {
                    panic!("admin required for disputed market");
                }

                market.status = MarketStatus::Resolved;
                Self::write_market(&env, &market);
            }
            _ => panic!("market cannot be finalized in current state"),
        }

        events::emit_resolution_finalized(
            &env,
            market_id_to_u64(&market.market_id),
            env.ledger().timestamp(),
        );
    }

    /// Returns the full [`Market`] struct for this contract.
    ///
    /// Read-only — does not modify state.
    ///
    /// # Arguments
    ///
    /// * `env` - The Soroban execution environment.
    ///
    /// # Returns
    ///
    /// Returns the [`Market`] stored in this contract.
    ///
    /// # Panics
    ///
    /// Panics if the market has not been initialized.
    pub fn get_market_info(env: Env) -> Market {
        Self::read_market(&env)
    }

    /// Returns the [`Bet`] identified by `bet_id`.
    ///
    /// Read-only — does not modify state.
    ///
    /// # Arguments
    ///
    /// * `env` - The Soroban execution environment.
    /// * `bet_id` - Unique identifier of the bet to retrieve.
    ///
    /// # Returns
    ///
    /// Returns the [`Bet`] struct associated with `bet_id`.
    ///
    /// # Panics
    ///
    /// Panics if `bet_id` does not correspond to any recorded bet.
    pub fn get_bet(env: Env, bet_id: Bytes) -> Bet {
        env.storage().persistent().get(&DataKey::Bet(bet_id))
            .expect("bet not found")
    }

    /// Returns all bets placed by `bettor` in this market.
    ///
    /// Read-only — does not modify state.
    ///
    /// # Arguments
    ///
    /// * `env` - The Soroban execution environment.
    /// * `bettor` - Address whose bets should be retrieved.
    ///
    /// # Returns
    ///
    /// Returns a [`Vec<Bet>`] containing all bets placed by `bettor`.
    /// Returns an empty `Vec` if `bettor` has no bets in this market.
    pub fn get_bets_by_address(env: Env, bettor: Address) -> Vec<Bet> {
        let bet_ids: Vec<Bytes> = env.storage().persistent()
            .get(&DataKey::BetsByAddr(bettor))
            .unwrap_or(Vec::new(&env));
        let mut bets = Vec::new(&env);
        for id in bet_ids.iter() {
            if let Some(bet) = env.storage().persistent().get(&DataKey::Bet(id)) {
                bets.push_back(bet);
            }
        }
        bets
    }

    /// Estimates the payout for a bet based on current pool sizes.
    ///
    /// Uses the same formula as [`claim_winnings`] but does not modify state.
    /// Intended for frontend display of live payout estimates before market resolution.
    ///
    /// Pure view function that computes a user's stake plus their proportional share
    /// of the losing pool, minus applicable fees. Handles zero losing pool (payout == stake)
    /// without division-by-zero. Uses checked/saturating arithmetic - no overflow panics.
    ///
    /// # Arguments
    ///
    /// * `env` - The Soroban execution environment.
    /// * `bet_id` - Unique identifier of the bet to estimate.
    ///
    /// # Returns
    ///
    /// Returns the estimated payout in stroops, given current pool totals.
    ///
    /// # Panics
    ///
    /// Panics if `bet_id` does not correspond to any recorded bet.
    pub fn calculate_payout(env: Env, bet_id: Bytes) -> i128 {
        let bet: Bet = env.storage().persistent()
            .get(&DataKey::Bet(bet_id))
            .expect("bet not found");

        let market: Market = env.storage().persistent()
            .get(&DataKey::MarketInfo)
            .expect("market not initialized");

        let outcome = match market.outcome.outcome() {
            Some(o) => o,
            None => return 0,
        };

        let is_winner = match (&bet.side, &outcome) {
            (BetSide::FighterA, Outcome::FighterA) => true,
            (BetSide::FighterB, Outcome::FighterB) => true,
            _ => false,
        };

        if !is_winner {
            return 0;
        }

        let winning_pool = match outcome {
            Outcome::FighterA => market.pool_a,
            Outcome::FighterB => market.pool_b,
            _ => market.pool_a.checked_add(market.pool_b).expect("pool sum overflow"),
        };

        if winning_pool == 0 {
            return 0;
        }

        let fee_amount = shared::types::calculate_fee(market.total_pool, market.protocol_fee_bp);
        let net_pool = market.total_pool.checked_sub(fee_amount).expect("net pool underflow");
        bet.amount
            .checked_mul(net_pool)
            .expect("payout overflow")
            .checked_div(winning_pool)
            .expect("payout div zero")
    }

    /// Returns current pool sizes and implied odds for both fighters.
    ///
    /// Implied odds are expressed in basis points (0–10000), where
    /// `implied_odds_a = pool_a / total_pool * 10000`. When `total_pool` is zero,
    /// returns a 50/50 split (5000, 5000). Read-only — does not modify state.
    ///
    /// # Arguments
    ///
    /// * `env` - The Soroban execution environment.
    ///
    /// # Returns
    ///
    /// Returns a tuple `(pool_a, pool_b, implied_odds_a, implied_odds_b)` where:
    /// - `pool_a` / `pool_b` are total XLM staked per side, in stroops.
    /// - `implied_odds_a` / `implied_odds_b` are basis-point probabilities summing to 10000.
    pub fn get_pool_odds(env: Env) -> (i128, i128, u32, u32) {
        let market: Market = env.storage().persistent()
            .get(&DataKey::MarketInfo)
            .expect("market not initialized");
        let total = market.pool_a.checked_add(market.pool_b).unwrap_or(0);
        let (odds_a, odds_b) = if total == 0 {
            (5_000u32, 5_000u32)
        } else {
            let a = market.pool_a
                .checked_mul(10_000)
                .expect("odds multiplication overflow")
                .checked_div(total)
                .expect("odds division error") as u32;
            (a, 10_000u32.checked_sub(a).expect("odds underflow"))
        };
        (market.pool_a, market.pool_b, odds_a, odds_b)
    }

    /// Returns complete market data including status, pools, and metadata.
    ///
    /// Read-only — does not modify state. Returns the full Market struct containing
    /// fighter information, pools, fees, status, and outcome.
    ///
    /// # Arguments
    ///
    /// * `env` - The Soroban execution environment.
    ///
    /// # Returns
    ///
    /// Returns the complete [`Market`] struct for this contract.
    ///
    /// # Panics
    ///
    /// Panics if the market has not been initialized.
    pub fn get_market_data(env: Env) -> Market {
        env.storage().persistent()
            .get(&DataKey::MarketInfo)
            .expect("market not initialized")
    }

    /// Returns a specific bet placed by an address, or None if not found.
    ///
    /// Read-only — does not modify state. Retrieves a bet by its ID and returns
    /// None if the address does not have a bet with that ID.
    ///
    /// # Arguments
    ///
    /// * `env` - The Soroban execution environment.
    /// * `bettor` - Address to query bets for.
    /// * `bet_id` - Unique identifier of the bet.
    ///
    /// # Returns
    ///
    /// Returns `Some(Bet)` if the bet exists and belongs to the address, or `None` otherwise.
    pub fn get_user_bet(env: Env, bettor: Address, bet_id: Bytes) -> Option<Bet> {
        if let Some(bet) = env.storage().persistent().get::<_, Bet>(&DataKey::Bet(bet_id)) {
            if bet.bettor == bettor {
                return Some(bet);
            }
        }
        None
    }

    /// Returns current pool totals for both fighters.
    ///
    /// Read-only — does not modify state. Returns the amount staked on each fighter
    /// and the total pool size.
    ///
    /// # Arguments
    ///
    /// * `env` - The Soroban execution environment.
    ///
    /// # Returns
    ///
    /// Returns a tuple `(pool_a, pool_b, total_pool)` where:
    /// - `pool_a` - Total XLM staked on Fighter A, in stroops.
    /// - `pool_b` - Total XLM staked on Fighter B, in stroops.
    /// - `total_pool` - Total XLM in all pools, in stroops.
    pub fn get_pool_totals(env: Env) -> (i128, i128, i128) {
        let market: Market = env.storage().persistent()
            .get(&DataKey::MarketInfo)
            .expect("market not initialized");
        (market.pool_a, market.pool_b, market.total_pool)
    }
}

// ─── TESTS ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    extern crate std;

    use super::*;
    use shared::event_parser::*;
    use std::string::ToString;
    use shared::test_utils::{event_count, last_event, last_event_name};
    use soroban_sdk::{
        contract, contractimpl,
        testutils::{Address as _, Ledger},
    };

    const NONCE: u64 = 7;
    const BETTING_ENDS_AT: u64 = 1_000;

    // ─── Mocks ────────────────────────────────────────────────────────────────

    #[contract]
    struct MockFactory;

    #[contractimpl]
    impl MockFactory {
        pub fn __constructor(env: Env, admin: Address) {
            env.storage().persistent().set(&Symbol::new(&env, "admin"), &admin);
        }

        pub fn get_config(env: Env) -> ProtocolConfig {
            let admin: Address = env.storage().persistent().get(&Symbol::new(&env, "admin")).unwrap();
            ProtocolConfig {
                admin: admin.clone(),
                fee_collector: admin,
                default_fee_bp: 200,
                min_bet_amount: 100,
                max_bet_amount: 1_000_000,
                dispute_window_sec: 86_400,
                paused: false,
            }
        }
    }

    /// Accepts escrow deposits without moving tokens.
    #[contract]
    struct MockTreasury;

    #[contractimpl]
    impl MockTreasury {
        pub fn deposit(_env: Env, _from_market: Address, _market_id: Bytes, _bettor: Address, _amount: i128) {}
    }

    // ─── Setup ────────────────────────────────────────────────────────────────

    struct Ctx {
        env: Env,
        client: MarketContractClient<'static>,
        admin: Address,
        oracle: Address,
        market_id: Bytes,
    }

    fn fighter(env: &Env, name: &str) -> Fighter {
        Fighter {
            name: String::from_str(env, name),
            record: String::from_str(env, "10-0"),
            nationality: String::from_str(env, "US"),
            weight_class: String::from_str(env, "Heavyweight"),
        }
    }

    /// Builds a market_id the same way MarketFactory::create_market does.
    fn factory_market_id(env: &Env, nonce: u64) -> Bytes {
        let mut id = [0xABu8; 32];
        id[0..8].copy_from_slice(&nonce.to_le_bytes());
        Bytes::from_array(env, &id)
    }

    fn setup() -> Ctx {
        let env = Env::default();
        env.mock_all_auths();

        let admin = Address::generate(&env);
        let oracle = Address::generate(&env);
        let factory = env.register(MockFactory, (admin.clone(),));
        let treasury = env.register(MockTreasury, ());
        let contract_id = env.register(MarketContract, ());
        let client = MarketContractClient::new(&env, &contract_id);
        let market_id = factory_market_id(&env, NONCE);

        client.initialize(
            &market_id,
            &fighter(&env, "Alpha"),
            &fighter(&env, "Beta"),
            &2_000u64,
            &BETTING_ENDS_AT,
            &oracle,
            &factory,
            &200u32,
            &Address::generate(&env),
            &86_400u64,
            &treasury,
            &Address::generate(&env),
        );

        Ctx { env, client, admin, oracle, market_id }
    }

    fn set_time(env: &Env, ts: u64) {
        env.ledger().with_mut(|li| li.timestamp = ts);
    }

    fn assert_last_event_name(env: &Env, name: &str) {
        assert_eq!(event_count(env), 1, "expected exactly one event");
        assert_eq!(last_event_name(env), Symbol::new(env, name));
    }

    fn is_snake_case(name: &str) -> bool {
        !name.is_empty()
            && name.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
    }

    fn resolved_ctx(outcome: Outcome) -> (Ctx, Address, Bytes) {
        let ctx = setup();
        let bettor = Address::generate(&ctx.env);
        let bet_id = ctx.client.place_bet(&bettor, &BetSide::FighterA, &500);
        set_time(&ctx.env, BETTING_ENDS_AT);
        ctx.client.lock_market(&ctx.oracle);
        ctx.client.resolve_market(&ctx.oracle, &outcome);
        (ctx, bettor, bet_id)
    }

    // ─── market_id topic ──────────────────────────────────────────────────────

    #[test]
    fn test_event_market_id_matches_factory_nonce() {
        let ctx = setup();
        assert_eq!(market_id_to_u64(&ctx.market_id), NONCE);
        let bettor = Address::generate(&ctx.env);
        ctx.client.place_bet(&bettor, &BetSide::FighterA, &500);
        let (topics, data) = last_event(&ctx.env);
        assert_eq!(parse_bet_placed_event(&ctx.env, &topics, &data).unwrap().market_id, NONCE);
    }

    // ─── Round trips per emitter ──────────────────────────────────────────────

    #[test]
    fn test_place_bet_emits_bet_placed() {
        let ctx = setup();
        let bettor = Address::generate(&ctx.env);
        set_time(&ctx.env, 10);
        let bet_id = ctx.client.place_bet(&bettor, &BetSide::FighterB, &750);

        assert_last_event_name(&ctx.env, "bet_placed");
        let (topics, data) = last_event(&ctx.env);
        let ev = parse_bet_placed_event(&ctx.env, &topics, &data).unwrap();
        assert_eq!(ev.market_id, NONCE);
        assert_eq!(ev.bet.bet_id, bet_id);
        assert_eq!(ev.bet.bettor, bettor);
        assert_eq!(ev.bet.market_id, NONCE);
        assert_eq!(ev.bet.side, shared::types::BetSide::FighterB);
        assert_eq!(ev.bet.amount, 750);
        assert_eq!(ev.bet.placed_at, 10);
        assert!(!ev.bet.claimed);
    }

    #[test]
    fn test_lock_market_emits_market_locked() {
        let ctx = setup();
        set_time(&ctx.env, 400);
        ctx.client.lock_market(&ctx.oracle);

        assert_last_event_name(&ctx.env, "market_locked");
        let (topics, data) = last_event(&ctx.env);
        let ev = parse_market_locked_event(&ctx.env, &topics, &data).unwrap();
        assert_eq!(ev.market_id, NONCE);
        assert_eq!(ev.locked_at, 400);
    }

    #[test]
    fn test_resolve_market_emits_market_resolved() {
        let ctx = setup();
        set_time(&ctx.env, BETTING_ENDS_AT);
        ctx.client.lock_market(&ctx.oracle);
        set_time(&ctx.env, 2_500);
        ctx.client.resolve_market(&ctx.oracle, &Outcome::FighterA);

        assert_last_event_name(&ctx.env, "market_resolved");
        let (topics, data) = last_event(&ctx.env);
        let ev = parse_market_resolved_event(&ctx.env, &topics, &data).unwrap();
        assert_eq!(ev.market_id, NONCE);
        assert_eq!(ev.outcome, shared::types::Outcome::FighterA);
        assert_eq!(ev.resolved_at, 2_500);
    }

    #[test]
    fn test_cancel_market_emits_market_cancelled() {
        let ctx = setup();
        ctx.client.cancel_market(&ctx.admin);

        assert_last_event_name(&ctx.env, "market_cancelled");
        let (topics, data) = last_event(&ctx.env);
        let ev = parse_market_cancelled_event(&ctx.env, &topics, &data).unwrap();
        assert_eq!(ev.market_id, NONCE);
        assert_eq!(ev.reason, String::from_str(&ctx.env, "cancelled_by_admin"));
    }

    #[test]
    fn test_dispute_resolution_emits_resolution_disputed() {
        let (ctx, bettor, _) = resolved_ctx(Outcome::FighterB);
        let reason = Bytes::from_slice(&ctx.env, b"wrong winner");
        ctx.client.dispute_resolution(&bettor, &reason);

        assert_last_event_name(&ctx.env, "resolution_disputed");
        let (topics, data) = last_event(&ctx.env);
        let ev = parse_resolution_disputed_event(&ctx.env, &topics, &data).unwrap();
        assert_eq!(ev.market_id, NONCE);
        assert_eq!(ev.disputer, bettor);
        assert_eq!(ev.reason, reason);
    }

    #[test]
    fn test_resolve_dispute_emits_dispute_resolved() {
        let (ctx, bettor, _) = resolved_ctx(Outcome::FighterB);
        ctx.client.dispute_resolution(&bettor, &Bytes::from_slice(&ctx.env, b"x"));
        ctx.client.resolve_dispute(&ctx.admin, &Outcome::FighterA);

        assert_last_event_name(&ctx.env, "dispute_resolved");
        let (topics, data) = last_event(&ctx.env);
        let ev = parse_dispute_resolved_event(&ctx.env, &topics, &data).unwrap();
        assert_eq!(ev.market_id, NONCE);
        assert_eq!(ev.final_outcome, shared::types::Outcome::FighterA);
    }

    #[test]
    fn test_claim_refund_emits_refund_claimed() {
        let ctx = setup();
        let bettor = Address::generate(&ctx.env);
        let bet_id = ctx.client.place_bet(&bettor, &BetSide::FighterA, &600);
        ctx.client.cancel_market(&ctx.admin);
        ctx.client.claim_refund(&bettor, &bet_id);

        assert_last_event_name(&ctx.env, "refund_claimed");
        let (topics, data) = last_event(&ctx.env);
        let ev = parse_refund_claimed_event(&ctx.env, &topics, &data).unwrap();
        assert_eq!(ev.market_id, NONCE);
        assert_eq!(ev.bettor, bettor);
        assert_eq!(ev.bet_id, bet_id);
        assert_eq!(ev.amount, 600);
    }

    #[test]
    fn test_claim_winnings_emits_winnings_claimed() {
        let (ctx, bettor, bet_id) = resolved_ctx(Outcome::FighterA);
        let payout = ctx.client.claim_winnings(&bettor, &bet_id);

        assert_last_event_name(&ctx.env, "winnings_claimed");
        let (topics, data) = last_event(&ctx.env);
        let ev = parse_winnings_claimed_event(&ctx.env, &topics, &data).unwrap();
        assert_eq!(ev.market_id, NONCE);
        assert_eq!(ev.receipt.bet_id, bet_id);
        assert_eq!(ev.receipt.bettor, bettor);
        assert_eq!(ev.receipt.payout, payout);
    }

    #[test]
    fn test_finalize_resolution_emits_resolution_finalized() {
        let (ctx, _, _) = resolved_ctx(Outcome::FighterA);
        set_time(&ctx.env, BETTING_ENDS_AT + 86_401);
        ctx.client.finalize_resolution(&None);

        assert_last_event_name(&ctx.env, "resolution_finalized");
        let (topics, data) = last_event(&ctx.env);
        let ev = parse_resolution_finalized_event(&ctx.env, &topics, &data).unwrap();
        assert_eq!(ev.market_id, NONCE);
        assert_eq!(ev.finalized_at, BETTING_ENDS_AT + 86_401);
    }

    // ─── Topic casing ─────────────────────────────────────────────────────────

    #[test]
    fn test_all_market_event_topics_are_snake_case() {
        let (ctx, bettor, bet_id) = resolved_ctx(Outcome::FighterA);
        let mut names: std::vec::Vec<std::string::String> = std::vec::Vec::new();
        let mut record = |env: &Env| names.push(last_event_name(env).to_string());

        record(&ctx.env); // market_resolved
        ctx.client.claim_winnings(&bettor, &bet_id);
        record(&ctx.env);
        ctx.client.dispute_resolution(&bettor, &Bytes::from_slice(&ctx.env, b"x"));
        record(&ctx.env);
        ctx.client.resolve_dispute(&ctx.admin, &Outcome::FighterA);
        record(&ctx.env);

        let other = setup();
        let b = Address::generate(&other.env);
        let id = other.client.place_bet(&b, &BetSide::FighterA, &500);
        record(&other.env);
        other.client.lock_market(&other.oracle);
        record(&other.env);
        other.client.cancel_market(&other.admin);
        record(&other.env);
        other.client.claim_refund(&b, &id);
        record(&other.env);

        assert_eq!(names.len(), 8);
        for name in names.iter() {
            assert!(is_snake_case(name), "topic `{}` is not snake_case", name);
        }
    }
}
