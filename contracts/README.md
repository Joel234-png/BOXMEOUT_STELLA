# BOXMEOUT Smart Contracts

This directory contains the core smart contracts for the BOXMEOUT boxing prediction market on Stellar/Soroban.

## Contracts

### Treasury
Manages accumulated protocol fees and implements fee collection/withdrawal logic.

**Key Methods:**
- `initialize(admin, factory, token)` - One-time setup
- `set_fee_bps(admin, fee_bps)` - Update protocol fee rate (admin-only)
- `deposit_fees(market_id, amount)` - Receive fees from markets
- `withdraw_fees(admin, recipient, amount)` - Withdraw accrued fees
- `emergency_drain(admin, recipient)` - Emergency drain when paused
- `get_balance()` - Current escrow balance
- `get_fee_bps()` - Current fee rate
- `get_total_fees_earned()` - Lifetime fees
- `get_withdrawal_log()` - History of withdrawals

### Market
Manages individual boxing prediction markets, bet placement, and claim resolution.

**Key Methods:**
- `initialize(...)` - Create a new market
- `place_bet(bettor, side, amount)` - Place a bet on a fighter
- `lock_market(oracle)` - Close market to new bets
- `resolve_market(oracle, outcome)` - Set final outcome
- `claim_winnings(bettor, bet_id)` - Claim winnings or refund

### MarketFactory
Deploys new Market contracts and manages protocol configuration.

**Key Methods:**
- `initialize(admin, treasury, oracle)` - Setup factory
- `create_market(...)` - Deploy a new market contract
- `get_config()` - Return current protocol config
- `update_config(admin, ...)` - Update protocol settings

### Shared
Common types, utilities, and constants used across contracts.

## Events Reference

### market_created
Emitted when a new market contract is initialized.

**Topics:**
- `Symbol("market_created")` - Event name
- `Bytes` - Market ID

**Data:**
- Full `Market` struct with all fields:
  - `market_id: Bytes` - Unique market identifier
  - `fighter_a: Fighter` - First fighter details (name, record, nationality, weight_class)
  - `fighter_b: Fighter` - Second fighter details
  - `scheduled_at: u64` - Unix timestamp of scheduled fight time
  - `betting_ends_at: u64` - Unix timestamp after which no bets accepted
  - `created_at: u64` - Timestamp when market was created
  - `created_by: Address` - Factory address that created this market
  - `status: MarketStatus` - Market status (Open)
  - `pool_a: i128` - Total bets on fighter A (0 at creation)
  - `pool_b: i128` - Total bets on fighter B (0 at creation)
  - `total_pool: i128` - Total pool size (0 at creation)
  - `protocol_fee_bp: u32` - Protocol fee in basis points
  - `oracle_address: Address` - Authorized oracle for resolution
  - `outcome: Option<Outcome>` - Final outcome (None at creation)
  - `fee_collector_address: Address` - Recipient of protocol fees
  - `resolved_at: u64` - Timestamp when resolved (0 at creation)
  - `dispute_window_sec: u64` - Duration of dispute period

### fee_deposited
Emitted when a market deposits accumulated protocol fees into treasury
(via `shared::events::emit_fee_deposited`).

**Topics:**
- `Symbol("fee_deposited")` - Event name

**Data:**
- `Address` - Market address depositing fees
- `Address` - Token address
- `i128` - Amount deposited (stroops)

### FeeBpsUpdated
Emitted when the treasury admin updates the protocol fee rate.

**Topics:**
- `Symbol("FeeBpsUpdated")` - Event name

**Data:**
- `u32` - New fee rate in basis points

### fee_withdrawn
Emitted when fees are withdrawn from the treasury
(via `shared::events::emit_fee_withdrawn`).

**Topics:**
- `Symbol("fee_withdrawn")` - Event name

**Data:**
- `Address` - Token withdrawn
- `i128` - Amount withdrawn (stroops)
- `Address` - Recipient of withdrawn fees

### emergency_drain
Emitted when the treasury is drained during a protocol pause
(via `shared::events::emit_emergency_drain`).

**Topics:**
- `Symbol("emergency_drain")` - Event name

**Data:**
- `Address` - Token drained
- `i128` - Total amount drained (stroops)
- `Address` - Admin executing the drain

### BetPlaced
Emitted when a bettor places a bet on a market.

**Topics:**
- `Symbol("bet_placed")` - Event name
- `Bytes` - Market ID

**Data:**
- `BetPlacedEvent` struct containing:
  - `bet_id: Bytes` - Unique bet identifier
  - `market_id: Bytes` - Market identifier
  - `bettor: Address` - Address that placed the bet
  - `side: BetSide` - Fighter chosen (FighterA or FighterB)
  - `amount: i128` - Bet amount in stroops
  - `placed_at: u64` - Timestamp of bet placement

## Storage Keys

### Treasury Storage
- `"ADMIN"` → `Address` - Treasury administrator
- `"FACTORY"` → `Address` - MarketFactory contract address
- `"TOKEN"` → `Address` - XLM token contract address
- `"BALANCE"` → `i128` - Current treasury balance
- `"TOTAL_FEES"` → `i128` - Cumulative fees received (never decremented)
- `"FEE_BPS"` → `u32` - Protocol fee rate in basis points
- `"WITHDRAWAL_LOG"` → `Vec<(Address, i128, u64)>` - History of fee withdrawals

### Market Storage
- `DataKey::MarketInfo` → `Market` - Current market state
- `DataKey::Factory` → `Address` - MarketFactory address
- `DataKey::Bet(bet_id)` → `Bet` - Individual bet record
- `DataKey::BetsByAddr(address)` → `Vec<Bytes>` - Bet IDs for an address
- `DataKey::Claimed(bet_id)` → `bool` - Whether bet has been claimed
- `DataKey::DisputeRaised` → `bool` - Whether market is under dispute
- `DataKey::DisputeReason` → `Bytes` - Reason for dispute
- `"BET_COUNT"` → `u64` - Total bets placed on this market

## Error Handling

### Common Panics

**Authorization:**
- `"not admin"` - Caller is not the stored admin address
- `"unauthorized: caller is not a registered market"` - Market not registered in factory
- `"not initialized"` - Contract called before initialization

**State Validation:**
- `"already initialized"` - Initialize called more than once
- `"amount exceeds balance"` - Withdrawal exceeds available balance
- `"fee_bps exceeds ceiling"` - Fee rate > 10000 basis points (100%)
- `"protocol is not paused"` - Emergency drain called while protocol active
- `"market not open"` - Bet placed on closed market
- `"betting period has ended"` - Bet placed after betting_ends_at

## Type Definitions

### MarketStatus
- `Open` - Bets are being accepted
- `Locked` - Fight started, no more bets
- `Resolved` - Winner declared, claims open
- `Cancelled` - Fight cancelled, full refunds
- `Disputed` - Result under review, claims frozen

### BetSide
- `FighterA` - Bet on first fighter
- `FighterB` - Bet on second fighter

### Outcome
- `FighterA` - Fighter A wins
- `FighterB` - Fighter B wins
- `Draw` - Match ends in draw
- `NoContest` - DQ or injury ruling
