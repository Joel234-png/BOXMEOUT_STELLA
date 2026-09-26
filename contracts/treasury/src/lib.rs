#![no_std]
use shared::{events, types::ProtocolConfig};
use soroban_sdk::{
    contract, contractimpl, contracttype, symbol_short, token, Address, Bytes, Env, String, Symbol,
    Vec,
};

// ─── STORAGE KEYS ─────────────────────────────────────────────────────────────
// "ADMIN"           -> Address
// "PENDING_ADMIN"   -> Address  (two-step admin rotation — C-65)
// "FACTORY"         -> Address
// "TOKEN"           -> Address  (XLM token contract)
// "FEE_BPS"         -> u32 (fee in basis points)
// "FEE_RECIPIENT"   -> Address
// "BALANCE"         -> i128
// "TOTAL_FEES"      -> i128
// "WITHDRAWAL_LOG"  -> Vec<(Address, i128, u64)>

fn key_admin(env: &Env) -> Symbol {
    Symbol::new(env, "ADMIN")
}

fn key_pending_admin(env: &Env) -> Symbol {
    Symbol::new(env, "PENDING_ADMIN")
}

fn key_factory(env: &Env) -> Symbol {
    Symbol::new(env, "FACTORY")
}

fn key_token(env: &Env) -> Symbol {
    Symbol::new(env, "TOKEN")
}

fn key_fee_bps(env: &Env) -> Symbol {
    Symbol::new(env, "FEE_BPS")
}

fn key_fee_recipient(env: &Env) -> Symbol {
    Symbol::new(env, "FEE_RECIPIENT")
}

fn key_balance(env: &Env) -> Symbol {
    Symbol::new(env, "BALANCE")
}

fn key_total_fees(env: &Env) -> Symbol {
    Symbol::new(env, "TOTAL_FEES")
}

fn key_wlog(env: &Env) -> Symbol {
    Symbol::new(env, "WITHDRAWAL_LOG")
}

#[contract]
pub struct Treasury;

#[contractimpl]
impl Treasury {
    /// Initializes the Treasury with admin, fee configuration, and token address.
    ///
    /// Must be called once immediately after deployment. Stores admin address,
    /// fee basis points, fee recipient, token address, and initializes balance
    /// tracking and withdrawal log.
    ///
    /// # Arguments
    ///
    /// * `env` - The Soroban execution environment.
    /// * `admin` - Address of the treasury administrator, authorized to withdraw funds.
    /// * `fee_bps` - Protocol fee in basis points (e.g., 200 = 2%). Must not exceed 1000 (10%).
    /// * `fee_recipient` - Address that receives protocol fees.
    /// * `factory` - Address of the `MarketFactory` contract.
    /// * `token` - Address of the XLM token contract.
    ///
    /// # Panics
    ///
    /// Panics if:
    /// - The treasury has already been initialized.
    /// - `fee_bps` exceeds 1000 (10%).
    pub fn initialize(
        env: Env,
        admin: Address,
        fee_bps: u32,
        fee_recipient: Address,
        factory: Address,
        token: Address,
    ) {
        if env.storage().persistent().has(&key_admin(&env)) {
            panic!("already initialized");
        }

        // Validate fee_bps does not exceed 10% (1000 basis points)
        if fee_bps > 1000 {
            panic!("fee_bps exceeds maximum of 1000 (10%)");
        }

        env.storage().persistent().set(&key_admin(&env), &admin);
        env.storage().persistent().set(&key_fee_bps(&env), &fee_bps);
        env.storage()
            .persistent()
            .set(&key_fee_recipient(&env), &fee_recipient);
        env.storage().persistent().set(&key_factory(&env), &factory);
        env.storage().persistent().set(&key_token(&env), &token);
        env.storage().persistent().set(&key_balance(&env), &0i128);
        env.storage()
            .persistent()
            .set(&key_total_fees(&env), &0i128);
        env.storage()
            .persistent()
            .set(&key_wlog(&env), &Vec::<(Address, i128, u64)>::new(&env));
    }

    /// Escrows a bettor's stake on behalf of a registered `Market` contract.
    ///
    /// Called by a `Market` contract when a bettor places a bet. Transfers
    /// `amount` of the configured bet token from `bettor` to this contract and
    /// credits the treasury balance. Emits a `bet_deposited` event.
    ///
    /// # Arguments
    ///
    /// * `env` - The Soroban execution environment.
    /// * `from_market` - Address of the Market contract making the deposit. Must authorize this call.
    /// * `market_id` - Identifier of the market the bet belongs to.
    /// * `bettor` - Address of the bettor whose funds are being escrowed.
    /// * `amount` - Amount of the bet token to escrow, in stroops.
    ///
    /// # Panics
    ///
    /// Panics if:
    /// - `from_market` has not authorized the call.
    /// - `from_market` does not match the address registered for `market_id` in the factory.
    pub fn deposit(env: Env, from_market: Address, market_id: Bytes, bettor: Address, amount: i128) {
        from_market.require_auth();

        let factory: Address = env
            .storage()
            .persistent()
            .get(&key_factory(&env))
            .expect("not initialized");

        let registered: Address = env.invoke_contract(
            &factory,
            &Symbol::new(&env, "get_market_address"),
            soroban_sdk::vec![&env, market_id.to_val()],
        );
        if registered != from_market {
            panic!("unauthorized: caller is not a registered market");
        }

        let token_addr: Address = env
            .storage()
            .persistent()
            .get(&key_token(&env))
            .expect("token not set");
        token::Client::new(&env, &token_addr).transfer(
            &bettor,
            &env.current_contract_address(),
            &amount,
        );

        let balance: i128 = env
            .storage()
            .persistent()
            .get(&key_balance(&env))
            .unwrap_or(0);
        env.storage()
            .persistent()
            .set(&key_balance(&env), &(balance + amount));

        events::emit_bet_deposited(&env, from_market, bettor, market_id, amount);
    }

    /// Receives protocol fees from a registered `Market` contract.
    ///
    /// Only callable by a Market contract address registered with the factory.
    /// Increments the treasury balance and emits a `fee_deposited` event.
    ///
    /// # Arguments
    ///
    /// * `env` - The Soroban execution environment.
    /// * `from_market` - Address of the Market contract depositing bets (must be authorized).
    /// * `market_id` - Identifier of the market, used for per-market escrow tracking.
    /// * `amount` - Amount of XLM to deposit into escrow, in stroops.
    ///
    /// # Panics
    ///
    /// Panics if:
    /// - The invoking contract address does not match the address registered for `market_id` in the factory.
    pub fn deposit_fees(env: Env, market_id: Bytes, amount: i128) {
        let factory: Address = env
            .storage()
            .persistent()
            .get(&key_factory(&env))
            .expect("not initialized");

        let caller = env.current_contract_address();

        let registered: Address = env.invoke_contract(
            &factory,
            &Symbol::new(&env, "get_market_address"),
            soroban_sdk::vec![&env, market_id.to_val()],
        );
        if registered != caller {
            panic!("unauthorized: caller is not a registered market");
        }

        let balance: i128 = env
            .storage()
            .persistent()
            .get(&key_balance(&env))
            .unwrap_or(0);
        let total: i128 = env
            .storage()
            .persistent()
            .get(&key_total_fees(&env))
            .unwrap_or(0);
        env.storage()
            .persistent()
            .set(&key_balance(&env), &(balance + amount));
        env.storage()
            .persistent()
            .set(&key_total_fees(&env), &(total + amount));

        let token_addr: Address = env
            .storage()
            .persistent()
            .get(&key_token(&env))
            .expect("token not set");
        events::emit_fee_deposited(&env, caller, token_addr, amount);
    }

    /// Transfers collected fees from the treasury to a recipient address.
    ///
    /// Validates that `amount ≤ BALANCE` and deducts it before transferring XLM.
    /// Appends an entry to `WITHDRAWAL_LOG`. Emits a `fee_withdrawn` event.
    ///
    /// # Arguments
    ///
    /// * `env` - The Soroban execution environment.
    /// * `admin` - Admin address. Must authorize this call.
    /// * `recipient` - Address that will receive the withdrawn XLM.
    /// * `amount` - Amount to withdraw in stroops. Must not exceed current `BALANCE`.
    ///
    /// # Panics
    ///
    /// Panics if:
    /// - `admin` has not authorized the call.
    /// - `amount` exceeds the current `BALANCE`.
    pub fn withdraw_fees(env: Env, admin: Address, recipient: Address, amount: i128) {
        admin.require_auth();

        let stored_admin: Address = env
            .storage()
            .persistent()
            .get(&key_admin(&env))
            .expect("not initialized");
        if stored_admin != admin {
            panic!("not admin");
        }

        let balance: i128 = env
            .storage()
            .persistent()
            .get(&key_balance(&env))
            .unwrap_or(0);
        if amount > balance {
            panic!("amount exceeds balance");
        }
        env.storage()
            .persistent()
            .set(&key_balance(&env), &(balance - amount));

        let token_addr: Address = env
            .storage()
            .persistent()
            .get(&key_token(&env))
            .expect("token not set");
        token::Client::new(&env, &token_addr).transfer(
            &env.current_contract_address(),
            &recipient,
            &amount,
        );

        let ts = env.ledger().timestamp();
        let mut log: Vec<(Address, i128, u64)> = env
            .storage()
            .persistent()
            .get(&key_wlog(&env))
            .unwrap_or(Vec::new(&env));
        log.push_back((recipient.clone(), amount, ts));
        env.storage().persistent().set(&key_wlog(&env), &log);

        events::emit_fee_withdrawn(&env, token_addr, amount, recipient);
    }

    /// Release escrowed winnings or refunds from a market to a bettor (issue #1181).
    /// Only callable by the registered market contract.
    pub fn release_winnings(env: Env, from_market: Address, market_id: Bytes, recipient: Address, amount: i128) {
        from_market.require_auth();

        let balance: i128 = env.storage().persistent().get(&key_balance(&env)).unwrap_or(0);
        if amount > balance {
            panic!("insufficient escrow for payout");
        }

        env.storage().persistent().set(&key_balance(&env), &(balance - amount));

        let token_addr: Address = env.storage().persistent().get(&key_token(&env)).expect("token not set");
        token::Client::new(&env, &token_addr).transfer(&env.current_contract_address(), &recipient, &amount);

        env.events().publish(
            (Symbol::new(&env, "WinningsReleased"),),
            (from_market, market_id, recipient, amount),
        );
    }

    /// Drains all treasury funds to `recipient` in an emergency.
    ///
    /// Only callable while the protocol is paused (verified via cross-contract call
    /// to the factory's `get_config`). Resets `BALANCE` to zero, logs the drain,
    /// and emits an `emergency_drain` event.
    ///
    /// # Arguments
    ///
    /// * `env` - The Soroban execution environment.
    /// * `admin` - Admin address. Must authorize this call.
    /// * `recipient` - Address that receives all drained XLM.
    ///
    /// # Returns
    ///
    /// Returns the total amount drained in stroops.
    ///
    /// # Panics
    ///
    /// Panics if:
    /// - `admin` has not authorized the call.
    /// - The protocol is not currently paused.
    pub fn emergency_drain(env: Env, admin: Address, recipient: Address) -> i128 {
        admin.require_auth();

        let stored_admin: Address = env
            .storage()
            .persistent()
            .get(&key_admin(&env))
            .expect("not initialized");
        if stored_admin != admin {
            panic!("not admin");
        }

        let factory: Address = env
            .storage()
            .persistent()
            .get(&key_factory(&env))
            .expect("factory not set");
        let config: ProtocolConfig = env.invoke_contract(
            &factory,
            &Symbol::new(&env, "get_config"),
            soroban_sdk::vec![&env],
        );
        if !config.paused {
            panic!("protocol is not paused");
        }

        let amount: i128 = env
            .storage()
            .persistent()
            .get(&key_balance(&env))
            .unwrap_or(0);

        let token_addr: Address = env
            .storage()
            .persistent()
            .get(&key_token(&env))
            .expect("token not set");
        token::Client::new(&env, &token_addr).transfer(
            &env.current_contract_address(),
            &recipient,
            &amount,
        );
        env.storage()
            .persistent()
            .set(&key_balance(&env), &0i128);

        let ts = env.ledger().timestamp();
        let mut log: Vec<(Address, i128, u64)> = env
            .storage()
            .persistent()
            .get(&key_wlog(&env))
            .unwrap_or(Vec::new(&env));
        log.push_back((recipient.clone(), amount, ts));
        env.storage().persistent().set(&key_wlog(&env), &log);

        events::emit_emergency_drain(&env, token_addr, amount, admin);

        amount
    }

    /// Returns the current treasury XLM balance.
    ///
    /// Read-only — does not modify state. Matches the sum of all deposits
    /// minus all withdrawals.
    ///
    /// # Returns
    ///
    /// Returns the current `BALANCE` in stroops. Returns `0` if never set.
    pub fn get_balance(env: Env) -> i128 {
        env.storage()
            .persistent()
            .get(&key_balance(&env))
            .unwrap_or(0)
    }

    /// Returns lifetime cumulative fees collected.
    ///
    /// Read-only — does not modify state.
    ///
    /// # Returns
    ///
    /// Returns the cumulative `TOTAL_FEES_EARNED` in stroops. Returns `0` if never set.
    pub fn get_total_fees_earned(env: Env) -> i128 {
        env.storage()
            .persistent()
            .get(&key_total_fees(&env))
            .unwrap_or(0)
    }

    /// Returns the complete log of all past withdrawals from the treasury.
    ///
    /// Each entry is a tuple of `(recipient, amount, timestamp)`. Read-only —
    /// does not modify state.
    ///
    /// # Returns
    ///
    /// Returns a [`Vec`] of `(Address, i128, u64)` tuples, one per withdrawal,
    /// in the order they occurred. Returns an empty `Vec` if no withdrawals have occurred.
    pub fn get_withdrawal_log(env: Env) -> Vec<(Address, i128, u64)> {
        env.storage()
            .persistent()
            .get(&key_wlog(&env))
            .unwrap_or(Vec::new(&env))
    }

    /// Returns the stored fee basis points.
    ///
    /// Read-only — does not modify state.
    ///
    /// # Returns
    ///
    /// Returns the `FEE_BPS` value set during initialization.
    pub fn get_fee_bps(env: Env) -> u32 {
        env.storage()
            .persistent()
            .get(&key_fee_bps(&env))
            .unwrap_or(0)
    }

    /// Returns the stored fee recipient address.
    ///
    /// Read-only — does not modify state.
    ///
    /// # Returns
    ///
    /// Returns the `FEE_RECIPIENT` address set during initialization.
    pub fn get_fee_recipient(env: Env) -> Address {
        env.storage()
            .persistent()
            .get(&key_fee_recipient(&env))
            .expect("not initialized")
    }

    // ─── C-65: Two-step admin rotation ────────────────────────────────────────

    /// Nominates `new_admin` as the pending administrator.
    ///
    /// Step 1 of the two-step rotation. The current admin proposes a successor;
    /// the successor must call [`accept_admin`] to finalise the transfer. Until
    /// that happens the current admin retains all privileges and the proposal
    /// can be overwritten by calling `propose_admin` again.
    ///
    /// # Arguments
    ///
    /// * `env`       - The Soroban execution environment.
    /// * `admin`     - Current admin address. Must authorize this call.
    /// * `new_admin` - Candidate address that will become the new admin.
    ///
    /// # Panics
    ///
    /// Panics if `admin` has not authorized the call or is not the stored admin.
    pub fn propose_admin(env: Env, admin: Address, new_admin: Address) {
        admin.require_auth();

        let stored_admin: Address = env
            .storage()
            .persistent()
            .get(&key_admin(&env))
            .expect("not initialized");
        if stored_admin != admin {
            panic!("not admin");
        }

        env.storage()
            .persistent()
            .set(&key_pending_admin(&env), &new_admin);

        env.events().publish(
            (Symbol::new(&env, "admin_proposed"),),
            (admin, new_admin, env.ledger().timestamp()),
        );
    }

    /// Completes the two-step admin rotation.
    ///
    /// Step 2 of the two-step rotation. The pending admin accepts the proposal,
    /// becoming the new admin. The `PENDING_ADMIN` entry is cleared on success.
    ///
    /// # Arguments
    ///
    /// * `env`           - The Soroban execution environment.
    /// * `pending_admin` - The address that was previously nominated. Must authorize this call.
    ///
    /// # Panics
    ///
    /// Panics if:
    /// - No pending admin has been proposed.
    /// - `pending_admin` has not authorized the call or does not match the stored pending admin.
    pub fn accept_admin(env: Env, pending_admin: Address) {
        pending_admin.require_auth();

        let stored_pending: Address = env
            .storage()
            .persistent()
            .get(&key_pending_admin(&env))
            .expect("no pending admin");
        if stored_pending != pending_admin {
            panic!("not pending admin");
        }

        let old_admin: Address = env
            .storage()
            .persistent()
            .get(&key_admin(&env))
            .expect("not initialized");

        env.storage()
            .persistent()
            .set(&key_admin(&env), &pending_admin);
        env.storage()
            .persistent()
            .remove(&key_pending_admin(&env));

        env.events().publish(
            (Symbol::new(&env, "admin_transferred"),),
            (old_admin, pending_admin, env.ledger().timestamp()),
        );
    }

    /// Updates the address that receives protocol fees.
    ///
    /// Admin-only. Emits a `fee_recipient_updated` event.
    ///
    /// # Arguments
    ///
    /// * `env`           - The Soroban execution environment.
    /// * `admin`         - Current admin address. Must authorize this call.
    /// * `new_recipient` - Address to receive future fee withdrawals.
    ///
    /// # Panics
    ///
    /// Panics if `admin` has not authorized the call or is not the stored admin.
    pub fn set_fee_recipient(env: Env, admin: Address, new_recipient: Address) {
        admin.require_auth();

        let stored_admin: Address = env
            .storage()
            .persistent()
            .get(&key_admin(&env))
            .expect("not initialized");
        if stored_admin != admin {
            panic!("not admin");
        }

        let old_recipient: Address = env
            .storage()
            .persistent()
            .get(&key_fee_recipient(&env))
            .expect("not initialized");

        env.storage()
            .persistent()
            .set(&key_fee_recipient(&env), &new_recipient);

        env.events().publish(
            (Symbol::new(&env, "fee_recipient_updated"),),
            (old_recipient, new_recipient, env.ledger().timestamp()),
        );
    }

    // ─── C-66: set_fee_bps ────────────────────────────────────────────────────

    /// Updates the protocol fee rate in basis points.
    ///
    /// Admin-only. Rejects values above 1000 (10%). Emits a `config_updated`
    /// event using the shared event helper so the indexer receives a uniform
    /// schema.
    ///
    /// # Arguments
    ///
    /// * `env`   - The Soroban execution environment.
    /// * `admin` - Current admin address. Must authorize this call.
    /// * `bps`   - New fee rate in basis points. Must be ≤ 1000.
    ///
    /// # Panics
    ///
    /// Panics if:
    /// - `admin` has not authorized the call or is not the stored admin.
    /// - `bps` exceeds 1000 (10%).
    pub fn set_fee_bps(env: Env, admin: Address, bps: u32) {
        admin.require_auth();

        let stored_admin: Address = env
            .storage()
            .persistent()
            .get(&key_admin(&env))
            .expect("not initialized");
        if stored_admin != admin {
            panic!("not admin");
        }

        if bps > 1000 {
            panic!("fee_bps exceeds maximum of 1000 (10%)");
        }

        env.storage().persistent().set(&key_fee_bps(&env), &bps);

        // Emit config_updated with param name "fee_bps" and the new value cast
        // to i128 so it fits the shared event schema (param_name: String, new_value: i128).
        env.events().publish(
            (Symbol::new(&env, "config_updated"),),
            (String::from_str(&env, "fee_bps"), bps as i128),
        );
    }
}

// ─── TESTS ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use shared::test_utils::{create_test_address, create_test_env};
    use soroban_sdk::IntoVal;

    #[test]
    fn test_initialize_success() {
        let env = create_test_env();
        let admin = create_test_address(&env);
        let factory = create_test_address(&env);
        let fee_recipient = create_test_address(&env);
        let token = create_test_address(&env);

        let contract_id = env.register_contract(None, Treasury);
        let client = TreasuryClient::new(&env, &contract_id);

        client.initialize(&admin, &200u32, &fee_recipient, &factory, &token);

        assert_eq!(client.get_balance(), 0);
        assert_eq!(client.get_total_fees_earned(), 0);
        assert_eq!(client.get_fee_bps(), 200);
        assert_eq!(client.get_withdrawal_log().len(), 0);
        assert_eq!(client.get_fee_recipient(), fee_recipient);
    }

    #[test]
    #[should_panic(expected = "already initialized")]
    fn test_double_initialize_panics() {
        let env = create_test_env();
        let admin = create_test_address(&env);
        let factory = create_test_address(&env);
        let fee_recipient = create_test_address(&env);
        let token = create_test_address(&env);

        let contract_id = env.register_contract(None, Treasury);
        let client = TreasuryClient::new(&env, &contract_id);

        client.initialize(&admin, &200u32, &fee_recipient, &factory, &token);
        client.initialize(&admin, &200u32, &fee_recipient, &factory, &token); // must panic
    }

    #[test]
    #[should_panic(expected = "fee_bps exceeds maximum of 1000 (10%)")]
    fn test_initialize_fee_bps_exceeds_maximum() {
        let env = create_test_env();
        let admin = create_test_address(&env);
        let factory = create_test_address(&env);
        let fee_recipient = create_test_address(&env);
        let token = create_test_address(&env);

        let contract_id = env.register_contract(None, Treasury);
        let client = TreasuryClient::new(&env, &contract_id);

        client.initialize(&admin, &1001u32, &fee_recipient, &factory, &token);
    }

    #[test]
    fn test_initialize_fee_bps_at_maximum() {
        let env = create_test_env();
        let admin = create_test_address(&env);
        let factory = create_test_address(&env);
        let fee_recipient = create_test_address(&env);
        let token = create_test_address(&env);

        let contract_id = env.register_contract(None, Treasury);
        let client = TreasuryClient::new(&env, &contract_id);

        client.initialize(&admin, &1000u32, &fee_recipient, &factory, &token);
        assert_eq!(client.get_fee_bps(), 1000);
    }

    // ─── withdraw_fees tests ───────────────────────────────────────────────────

    /// Helper: registers treasury, initialises with a funded token, and seeds
    /// BALANCE by directly setting storage so we can test withdraw_fees without
    /// needing a real market factory cross-contract call.
    fn setup_treasury_with_balance(env: &Env, balance: i128) -> (TreasuryClient, Address, Address) {
        use soroban_sdk::testutils::Ledger;

        env.ledger().with_mut(|li| li.timestamp = 1_000_000);

        let admin = create_test_address(env);
        let factory = create_test_address(env);
        let fee_recipient = create_test_address(env);

        // Mint `balance` tokens to the treasury contract so the token transfer succeeds.
        let contract_id = env.register_contract(None, Treasury);
        let token_addr = shared::test_utils::fund_address(env, &contract_id, balance);

        let client = TreasuryClient::new(env, &contract_id);
        client.initialize(&admin, &200u32, &fee_recipient, &factory, &token_addr);

        // Seed BALANCE via a direct storage write so we don't need the full
        // deposit_fees machinery (which requires a registered market).
        env.as_contract(&contract_id, || {
            env.storage().persistent().set(&key_balance(env), &balance);
            env.storage()
                .persistent()
                .set(&key_total_fees(env), &balance);
        });

        (client, admin, contract_id)
    }

    #[test]
    fn test_withdraw_fees_happy_path() {
        let env = create_test_env();
        env.mock_all_auths();

        let initial_balance: i128 = 10_000;
        let withdraw_amount: i128 = 3_000;
        let (client, admin, _) = setup_treasury_with_balance(&env, initial_balance);
        let recipient = create_test_address(&env);

        client.withdraw_fees(&admin, &recipient, &withdraw_amount);

        // Balance decremented correctly
        assert_eq!(client.get_balance(), initial_balance - withdraw_amount);

        // Withdrawal logged
        let log = client.get_withdrawal_log();
        assert_eq!(log.len(), 1);
        let entry = log.get(0).unwrap();
        assert_eq!(entry.0, recipient);
        assert_eq!(entry.1, withdraw_amount);
    }

    #[test]
    fn test_withdraw_fees_full_balance() {
        let env = create_test_env();
        env.mock_all_auths();

        let balance: i128 = 5_000;
        let (client, admin, _) = setup_treasury_with_balance(&env, balance);
        let recipient = create_test_address(&env);

        // Withdraw exactly the full balance — must succeed
        client.withdraw_fees(&admin, &recipient, &balance);

        assert_eq!(client.get_balance(), 0);
        assert_eq!(client.get_withdrawal_log().len(), 1);
    }

    #[test]
    #[should_panic(expected = "amount exceeds balance")]
    fn test_withdraw_fees_exceeds_balance_panics() {
        let env = create_test_env();
        env.mock_all_auths();

        let balance: i128 = 1_000;
        let (client, admin, _) = setup_treasury_with_balance(&env, balance);
        let recipient = create_test_address(&env);

        // Attempt to withdraw more than available — must panic
        client.withdraw_fees(&admin, &recipient, &(balance + 1));
    }

    #[test]
    #[should_panic(expected = "not admin")]
    fn test_withdraw_fees_non_admin_panics() {
        let env = create_test_env();
        env.mock_all_auths();

        let (client, _, _) = setup_treasury_with_balance(&env, 5_000);
        let non_admin = create_test_address(&env);
        let recipient = create_test_address(&env);

        // A random address that is not the stored admin must panic
        client.withdraw_fees(&non_admin, &recipient, &1_000);
    }

    #[test]
    fn test_withdraw_fees_multiple_withdrawals_logged() {
        let env = create_test_env();
        env.mock_all_auths();

        let (client, admin, _) = setup_treasury_with_balance(&env, 9_000);
        let recipient_a = create_test_address(&env);
        let recipient_b = create_test_address(&env);

        client.withdraw_fees(&admin, &recipient_a, &4_000);
        client.withdraw_fees(&admin, &recipient_b, &2_000);

        assert_eq!(client.get_balance(), 3_000);

        let log = client.get_withdrawal_log();
        assert_eq!(log.len(), 2);
        assert_eq!(log.get(0).unwrap().0, recipient_a);
        assert_eq!(log.get(0).unwrap().1, 4_000i128);
        assert_eq!(log.get(1).unwrap().0, recipient_b);
        assert_eq!(log.get(1).unwrap().1, 2_000i128);
    }

    #[test]
    #[should_panic(expected = "amount exceeds balance")]
    fn test_withdraw_fees_zero_balance_panics() {
        let env = create_test_env();
        env.mock_all_auths();

        let (client, admin, _) = setup_treasury_with_balance(&env, 0);
        let recipient = create_test_address(&env);

        // Withdrawing any positive amount from an empty treasury must panic
        client.withdraw_fees(&admin, &recipient, &1);
    }

    // ─── C-65: propose_admin / accept_admin ───────────────────────────────────

    #[test]
    fn test_propose_and_accept_admin() {
        let env = create_test_env();
        env.mock_all_auths();

        let (client, admin, _) = setup_treasury_with_balance(&env, 0);
        let new_admin = create_test_address(&env);

        // Propose
        client.propose_admin(&admin, &new_admin);

        // Accept — new_admin is now the stored admin
        client.accept_admin(&new_admin);

        // Confirm rotation: only new_admin can call set_fee_bps without panic
        client.set_fee_bps(&new_admin, &300u32);
        assert_eq!(client.get_fee_bps(), 300);
    }

    #[test]
    #[should_panic(expected = "not admin")]
    fn test_propose_admin_non_admin_panics() {
        let env = create_test_env();
        env.mock_all_auths();

        let (client, _, _) = setup_treasury_with_balance(&env, 0);
        let random = create_test_address(&env);
        let candidate = create_test_address(&env);

        client.propose_admin(&random, &candidate);
    }

    #[test]
    #[should_panic(expected = "not pending admin")]
    fn test_accept_admin_wrong_caller_panics() {
        let env = create_test_env();
        env.mock_all_auths();

        let (client, admin, _) = setup_treasury_with_balance(&env, 0);
        let new_admin = create_test_address(&env);
        let impostor = create_test_address(&env);

        client.propose_admin(&admin, &new_admin);
        client.accept_admin(&impostor);
    }

    #[test]
    #[should_panic(expected = "no pending admin")]
    fn test_accept_admin_without_proposal_panics() {
        let env = create_test_env();
        env.mock_all_auths();

        let (client, _, _) = setup_treasury_with_balance(&env, 0);
        let random = create_test_address(&env);

        client.accept_admin(&random);
    }

    // ─── C-65: set_fee_recipient ──────────────────────────────────────────────

    #[test]
    fn test_set_fee_recipient_updates_address() {
        let env = create_test_env();
        env.mock_all_auths();

        let (client, admin, _) = setup_treasury_with_balance(&env, 0);
        let new_recipient = create_test_address(&env);

        client.set_fee_recipient(&admin, &new_recipient);
        assert_eq!(client.get_fee_recipient(), new_recipient);
    }

    #[test]
    #[should_panic(expected = "not admin")]
    fn test_set_fee_recipient_non_admin_panics() {
        let env = create_test_env();
        env.mock_all_auths();

        let (client, _, _) = setup_treasury_with_balance(&env, 0);
        let random = create_test_address(&env);
        let new_recipient = create_test_address(&env);

        client.set_fee_recipient(&random, &new_recipient);
    }

    // ─── C-66: set_fee_bps ────────────────────────────────────────────────────

    #[test]
    fn test_set_fee_bps_updates_value() {
        let env = create_test_env();
        env.mock_all_auths();

        let (client, admin, _) = setup_treasury_with_balance(&env, 0);
        client.set_fee_bps(&admin, &500u32);
        assert_eq!(client.get_fee_bps(), 500);
    }

    #[test]
    fn test_set_fee_bps_at_maximum_boundary() {
        let env = create_test_env();
        env.mock_all_auths();

        let (client, admin, _) = setup_treasury_with_balance(&env, 0);
        client.set_fee_bps(&admin, &1000u32);
        assert_eq!(client.get_fee_bps(), 1000);
    }

    #[test]
    fn test_set_fee_bps_to_zero() {
        let env = create_test_env();
        env.mock_all_auths();

        let (client, admin, _) = setup_treasury_with_balance(&env, 0);
        client.set_fee_bps(&admin, &0u32);
        assert_eq!(client.get_fee_bps(), 0);
    }

    #[test]
    #[should_panic(expected = "fee_bps exceeds maximum of 1000 (10%)")]
    fn test_set_fee_bps_above_max_panics() {
        let env = create_test_env();
        env.mock_all_auths();

        let (client, admin, _) = setup_treasury_with_balance(&env, 0);
        client.set_fee_bps(&admin, &1001u32);
    }

    #[test]
    #[should_panic(expected = "not admin")]
    fn test_set_fee_bps_non_admin_panics() {
        let env = create_test_env();
        env.mock_all_auths();

        let (client, _, _) = setup_treasury_with_balance(&env, 0);
        let random = create_test_address(&env);
        client.set_fee_bps(&random, &100u32);
    }
}
