#![no_std]
//! ============================================================
//! BOXMEOUT — MarketFactory Contract
//! Deploys and tracks Market contract instances.
//! ============================================================
use soroban_sdk::{contract, contractimpl, Address, Bytes, BytesN, Env, Map, String, Symbol, Vec};

use shared::{errors::ContractError, types::{MarketInfo, ProtocolConfig}};

// Storage keys for persistent state
const ADMIN: &str = "ADMIN";
const MARKET_WASM_HASH: &str = "MARKET_WASM_HASH";
const TREASURY: &str = "TREASURY";
const PAUSED: &str = "PAUSED";
const MARKET_COUNT_KEY: &str = "MARKET_COUNT";
const MARKET_MAP: &str = "MARKET_MAP";
const ALL_MARKETS_KEY: &str = "ALL_MARKETS";
const CONFIG_KEY: &str = "CONFIG";

/// Maximum number of markets that may be returned in a single `list_markets` /
/// `list_active_markets` page, regardless of the caller-requested `limit`.
const MAX_PAGE_SIZE: u32 = 50;

#[contract]
pub struct MarketFactory;

#[contractimpl]
impl MarketFactory {
    /// One-time setup. Stores the admin, the Market contract's wasm hash
    /// (used for all future deployments), and the treasury address.
    ///
    /// # Errors
    /// Returns `ContractError::AlreadyInitialized` if called more than once.
    pub fn initialize(
        env: Env,
        admin: Address,
        market_wasm_hash: BytesN<32>,
        treasury: Address,
        fee_collector: Address,
        default_fee_bp: u32,
        min_bet_amount: i128,
        max_bet_amount: i128,
        dispute_window_sec: u64,
    ) -> Result<(), ContractError> {
        if env.storage().persistent().has(&ADMIN) {
            return Err(ContractError::AlreadyInitialized);
        }

        let config = ProtocolConfig {
            admin: admin.clone(),
            fee_collector,
            default_fee_bp,
            min_bet_amount,
            max_bet_amount,
            dispute_window_sec,
            paused: false,
        };

        env.storage().persistent().set(&ADMIN, &admin);
        env.storage().persistent().set(&MARKET_WASM_HASH, &market_wasm_hash);
        env.storage().persistent().set(&TREASURY, &treasury);
        env.storage().persistent().set(&PAUSED, &false);
        env.storage().persistent().set(&CONFIG_KEY, &config);
        env.storage().persistent().set(&MARKET_COUNT_KEY, &0u64);
        env.storage()
            .persistent()
            .set(&ALL_MARKETS_KEY, &Vec::<Bytes>::new(&env));
        env.storage()
            .persistent()
            .set(&MARKET_MAP, &Map::<Bytes, MarketInfo>::new(&env));

        Ok(())
    }

    /// Updates the Market wasm hash used for new deployments.
    ///
    /// Only the protocol admin can call this. Only affects markets deployed
    /// after this call — already-deployed Market instances keep running the
    /// wasm code they were originally deployed with.
    pub fn upgrade_market_wasm(
        env: Env,
        admin: Address,
        new_wasm_hash: BytesN<32>,
    ) -> Result<(), ContractError> {
        admin.require_auth();

        let stored_admin: Address = env.storage().persistent()
            .get(&ADMIN)
            .ok_or(ContractError::Unauthorized)?;

        if admin != stored_admin {
            return Err(ContractError::Unauthorized);
        }

        env.storage().persistent().set(&MARKET_WASM_HASH, &new_wasm_hash);
        env.events().publish(("market_wasm_upgraded",), new_wasm_hash);

        Ok(())
    }

    /// Returns the stored Market contract wasm hash.
    pub fn get_market_wasm_hash(env: Env) -> BytesN<32> {
        env.storage()
            .persistent()
            .get(&MARKET_WASM_HASH)
            .expect("not initialized")
    }

    /// Returns the stored treasury address.
    pub fn get_treasury(env: Env) -> Address {
        env.storage().persistent().get(&TREASURY).expect("not initialized")
    }

    /// Returns the stored protocol configuration.
    pub fn get_config(env: Env) -> Result<ProtocolConfig, ContractError> {
        env.storage()
            .persistent()
            .get(&CONFIG_KEY)
            .ok_or(ContractError::Unauthorized)
    }

    /// Deploys a new Market contract instance and registers its `MarketInfo`.
    ///
    /// # Errors
    /// - `ContractError::FactoryPaused` if the factory is paused
    /// - `ContractError::InvalidTimestamp` if `end_time` is in the past, or
    ///   `lock_time` is after `end_time`
    pub fn create_market(
        env: Env,
        caller: Address,
        fighter_a: String,
        fighter_b: String,
        oracle: Address,
        lock_time: u64,
        end_time: u64,
    ) -> Result<Bytes, ContractError> {
        caller.require_auth();

        let paused: bool = env.storage().persistent().get(&PAUSED).unwrap_or(false);
        if paused {
            return Err(ContractError::FactoryPaused);
        }

        let now = env.ledger().timestamp();
        if end_time <= now || lock_time > end_time {
            return Err(ContractError::InvalidTimestamp);
        }

        let count: u64 = env.storage().persistent().get(&MARKET_COUNT_KEY).unwrap_or(0);

        // Generate a collision-resistant market_id from the creation nonce,
        // both fighter names, and the scheduled end_time.
        let mut id_bytes = [0u8; 32];
        id_bytes[0..8].copy_from_slice(&count.to_le_bytes());
        for (i, byte) in fighter_a.to_bytes().iter().take(8).enumerate() {
            id_bytes[8 + i] ^= byte;
        }
        for (i, byte) in fighter_b.to_bytes().iter().take(8).enumerate() {
            id_bytes[16 + i] ^= byte;
        }
        id_bytes[24..32].copy_from_slice(&end_time.to_le_bytes());
        let market_id = Bytes::from_array(&env, &id_bytes);

        let wasm_hash: BytesN<32> = env
            .storage()
            .persistent()
            .get(&MARKET_WASM_HASH)
            .expect("not initialized");

        let salt = BytesN::from_array(&env, &id_bytes);
        let market_address = env
            .deployer()
            .with_address(env.current_contract_address(), salt)
            .deploy_v2(wasm_hash, ());

        let config = Self::get_config(&env)?;
        let treasury = Self::get_treasury(&env);

        env.invoke_contract::<Result<(), ContractError>>(
            &market_address,
            &Symbol::new(&env, "initialize"),
            soroban_sdk::vec![
                &env,
                env.current_contract_address().into(),
                oracle.clone().into(),
                treasury.clone().into(),
                config.default_fee_bp.into(),
                lock_time.into(),
                end_time.into(),
            ],
        )?;

        let _: () = env.invoke_contract(
            &treasury,
            &Symbol::new(&env, "register_market"),
            soroban_sdk::vec![&env, market_address.clone().into()],
        );

        let info = MarketInfo {
            market_id: market_id.clone(),
            market_address,
            creator: caller,
            fighter_a: fighter_a.clone(),
            fighter_b: fighter_b.clone(),
            oracle: oracle.clone(),
            lock_time,
            end_time,
            created_at: now,
        };

        let mut market_map: Map<Bytes, MarketInfo> = env
            .storage()
            .persistent()
            .get(&MARKET_MAP)
            .unwrap_or_else(|| Map::new(&env));
        market_map.set(market_id.clone(), info.clone());
        env.storage().persistent().set(&MARKET_MAP, &market_map);

        let mut all_markets: Vec<Bytes> = env
            .storage()
            .persistent()
            .get(&ALL_MARKETS_KEY)
            .unwrap_or_else(|| Vec::new(&env));
        all_markets.push_back(market_id.clone());
        env.storage().persistent().set(&ALL_MARKETS_KEY, &all_markets);

        env.storage()
            .persistent()
            .set(&MARKET_COUNT_KEY, &(count + 1));

        env.events().publish(("market_created",), info);

        Ok(market_id)
    }

    /// Read-only lookup of a single market by ID. Does not mutate state.
    pub fn get_market(env: Env, market_id: Bytes) -> Option<MarketInfo> {
        let map: Map<Bytes, MarketInfo> = env
            .storage()
            .persistent()
            .get(&MARKET_MAP)
            .unwrap_or_else(|| Map::new(&env));
        map.get(market_id)
    }

    /// Returns a bounded, stably-ordered (creation order) page of all markets
    /// ever created. `limit` is capped at `MAX_PAGE_SIZE` regardless of the
    /// value requested, to bound gas.
    pub fn list_markets(env: Env, offset: u32, limit: u32) -> Vec<MarketInfo> {
        let all_ids: Vec<Bytes> = env
            .storage()
            .persistent()
            .get(&ALL_MARKETS_KEY)
            .unwrap_or_else(|| Vec::new(&env));

        let total = all_ids.len();
        if offset >= total {
            return Vec::new(&env);
        }

        let map: Map<Bytes, MarketInfo> = env
            .storage()
            .persistent()
            .get(&MARKET_MAP)
            .unwrap_or_else(|| Map::new(&env));

        let capped_limit = if limit > MAX_PAGE_SIZE { MAX_PAGE_SIZE } else { limit };
        let end = (offset + capped_limit).min(total);

        let mut result: Vec<MarketInfo> = Vec::new(&env);
        for i in offset..end {
            let id = all_ids.get(i).unwrap();
            result.push_back(map.get(id).unwrap());
        }
        result
    }

    /// Returns every market whose `end_time` has not yet passed, in creation order.
    pub fn list_active_markets(env: Env) -> Vec<MarketInfo> {
        let all_ids: Vec<Bytes> = env
            .storage()
            .persistent()
            .get(&ALL_MARKETS_KEY)
            .unwrap_or_else(|| Vec::new(&env));

        let map: Map<Bytes, MarketInfo> = env
            .storage()
            .persistent()
            .get(&MARKET_MAP)
            .unwrap_or_else(|| Map::new(&env));

        let now = env.ledger().timestamp();
        let mut result: Vec<MarketInfo> = Vec::new(&env);
        for id in all_ids.iter() {
            let info = map.get(id).unwrap();
            if info.end_time > now {
                result.push_back(info);
            }
        }
        result
    }

    /// Transfers admin rights to `new_admin`.
    ///
    /// # Errors
    /// Returns `ContractError::Unauthorized` if `current_admin` does not match
    /// the stored admin.
    pub fn set_admin(env: Env, current_admin: Address, new_admin: Address) -> Result<(), ContractError> {
        current_admin.require_auth();

        let admin: Address = env.storage().persistent().get(&ADMIN).expect("not initialized");
        if admin != current_admin {
            return Err(ContractError::Unauthorized);
        }

        env.storage().persistent().set(&ADMIN, &new_admin);
        Ok(())
    }

    /// Returns whether the factory is currently paused.
    pub fn is_paused(env: Env) -> bool {
        env.storage().persistent().get(&PAUSED).unwrap_or(false)
    }

    /// Pauses the factory. While paused, `create_market` reverts; existing
    /// markets are unaffected.
    ///
    /// # Errors
    /// Returns `ContractError::Unauthorized` if `admin` does not match the
    /// stored admin.
    pub fn pause_factory(env: Env, admin: Address) -> Result<(), ContractError> {
        Self::require_admin(&env, &admin)?;
        env.storage().persistent().set(&PAUSED, &true);
        Ok(())
    }

    /// Unpauses the factory, re-enabling `create_market`.
    ///
    /// # Errors
    /// Returns `ContractError::Unauthorized` if `admin` does not match the
    /// stored admin.
    pub fn unpause_factory(env: Env, admin: Address) -> Result<(), ContractError> {
        Self::require_admin(&env, &admin)?;
        env.storage().persistent().set(&PAUSED, &false);
        Ok(())
    }

    fn require_admin(env: &Env, admin: &Address) -> Result<(), ContractError> {
        admin.require_auth();
        let stored: Address = env.storage().persistent().get(&ADMIN).expect("not initialized");
        if stored != *admin {
            return Err(ContractError::Unauthorized);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::{testutils::{Address as _, Ledger as _}, Env};

    // A minimal, dependency-free contract wasm used only to exercise the real
    // `env.deployer()...deploy_v2()` path in tests. It is not the production
    // Market contract; it just needs to be *some* valid, deployable wasm.
    const DUMMY_WASM: &[u8] = include_bytes!("../test_fixtures/dummy.wasm");

    fn setup() -> (Env, MarketFactoryClient<'static>, Address, Address) {
        let env = Env::default();
        env.mock_all_auths();

        let contract_id = env.register(MarketFactory, ());
        let client = MarketFactoryClient::new(&env, &contract_id);
        let admin = Address::generate(&env);
        let treasury = Address::generate(&env);

        (env, client, admin, treasury)
    }

    fn init(env: &Env, client: &MarketFactoryClient<'static>, admin: &Address, treasury: &Address) {
        let wasm_hash = env.deployer().upload_contract_wasm(DUMMY_WASM);
        client.initialize(admin, &wasm_hash, treasury);
    }

    fn create_default_market(env: &Env, client: &MarketFactoryClient<'static>) -> Bytes {
        let caller = Address::generate(env);
        let oracle = Address::generate(env);
        let now = env.ledger().timestamp();
        client.create_market(
            &caller,
            &String::from_str(env, "Fighter A"),
            &String::from_str(env, "Fighter B"),
            &oracle,
            &(now + 100),
            &(now + 200),
        )
    }

    // ── C-02: initialize ──────────────────────────────────────────────────

    #[test]
    fn initialize_stores_admin_wasm_hash_and_treasury() {
        let (env, client, admin, treasury) = setup();
        init(&env, &client, &admin, &treasury);

        let wasm_hash = env.deployer().upload_contract_wasm(DUMMY_WASM);
        assert_eq!(client.get_admin(), admin);
        assert_eq!(client.get_treasury(), treasury);
        assert_eq!(client.get_market_wasm_hash(), wasm_hash);
        assert!(!client.is_paused());
    }

    #[test]
    fn initialize_twice_fails() {
        let (env, client, admin, treasury) = setup();
        init(&env, &client, &admin, &treasury);

        let wasm_hash = env.deployer().upload_contract_wasm(DUMMY_WASM);
        let result = client.try_initialize(&admin, &wasm_hash, &treasury);
        assert_eq!(result, Err(Ok(ContractError::AlreadyInitialized)));
    }

    // ── C-03: create_market ───────────────────────────────────────────────

    #[test]
    fn create_market_happy_path() {
        let (env, client, admin, treasury) = setup();
        init(&env, &client, &admin, &treasury);

        let market_id = create_default_market(&env, &client);
        let info = client.get_market(&market_id).unwrap();

        assert_eq!(info.market_id, market_id);
        assert_eq!(info.fighter_a, String::from_str(&env, "Fighter A"));
        assert_eq!(info.fighter_b, String::from_str(&env, "Fighter B"));
    }

    #[test]
    fn create_market_rejects_end_time_in_the_past() {
        let (env, client, admin, treasury) = setup();
        init(&env, &client, &admin, &treasury);

        let caller = Address::generate(&env);
        let oracle = Address::generate(&env);
        let now = env.ledger().timestamp();

        let result = client.try_create_market(
            &caller,
            &String::from_str(&env, "Fighter A"),
            &String::from_str(&env, "Fighter B"),
            &oracle,
            &now,
            &now, // end_time == now, not in the future
        );
        assert_eq!(result, Err(Ok(ContractError::InvalidTimestamp)));
    }

    #[test]
    fn create_market_rejects_lock_time_after_end_time() {
        let (env, client, admin, treasury) = setup();
        init(&env, &client, &admin, &treasury);

        let caller = Address::generate(&env);
        let oracle = Address::generate(&env);
        let now = env.ledger().timestamp();

        let result = client.try_create_market(
            &caller,
            &String::from_str(&env, "Fighter A"),
            &String::from_str(&env, "Fighter B"),
            &oracle,
            &(now + 200), // lock_time
            &(now + 100), // end_time, before lock_time
        );
        assert_eq!(result, Err(Ok(ContractError::InvalidTimestamp)));
    }

    // ── C-04: get_market ──────────────────────────────────────────────────

    #[test]
    fn get_market_unknown_id_returns_none() {
        let (env, client, admin, treasury) = setup();
        init(&env, &client, &admin, &treasury);

        let unknown_id = Bytes::from_array(&env, &[9u8; 32]);
        assert_eq!(client.get_market(&unknown_id), None);
    }

    #[test]
    fn get_market_known_id_returns_some() {
        let (env, client, admin, treasury) = setup();
        init(&env, &client, &admin, &treasury);

        let market_id = create_default_market(&env, &client);
        assert!(client.get_market(&market_id).is_some());
    }

    // ── C-05: list_markets ──────────────────────────────────────────────────

    #[test]
    fn list_markets_respects_offset_and_limit_in_creation_order() {
        let (env, client, admin, treasury) = setup();
        init(&env, &client, &admin, &treasury);

        let first = create_default_market(&env, &client);
        let second = create_default_market(&env, &client);
        let third = create_default_market(&env, &client);

        let page = client.list_markets(&1, &10);
        assert_eq!(page.len(), 2);
        assert_eq!(page.get(0).unwrap().market_id, second);
        assert_eq!(page.get(1).unwrap().market_id, third);

        let all = client.list_markets(&0, &10);
        assert_eq!(all.len(), 3);
        assert_eq!(all.get(0).unwrap().market_id, first);
    }

    #[test]
    fn list_markets_caps_limit_at_max_page_size() {
        let (env, client, admin, treasury) = setup();
        init(&env, &client, &admin, &treasury);

        for _ in 0..3 {
            create_default_market(&env, &client);
        }

        // Requesting far more than exist and more than MAX_PAGE_SIZE should
        // just return everything that exists, not panic or overflow.
        let page = client.list_markets(&0, &1_000);
        assert_eq!(page.len(), 3);
    }

    #[test]
    fn list_markets_out_of_range_offset_returns_empty() {
        let (env, client, admin, treasury) = setup();
        init(&env, &client, &admin, &treasury);

        create_default_market(&env, &client);

        let page = client.list_markets(&50, &10);
        assert_eq!(page.len(), 0);
    }

    // ── C-06: list_active_markets ───────────────────────────────────────────

    #[test]
    fn list_active_markets_filters_by_end_time() {
        let (env, client, admin, treasury) = setup();
        init(&env, &client, &admin, &treasury);

        // A market ending soon...
        let caller = Address::generate(&env);
        let oracle = Address::generate(&env);
        let now = env.ledger().timestamp();
        let soon_ending = client.create_market(
            &caller,
            &String::from_str(&env, "Fighter A"),
            &String::from_str(&env, "Fighter B"),
            &oracle,
            &(now + 10),
            &(now + 20),
        );
        // ...and one ending much later.
        let later_ending = client.create_market(
            &caller,
            &String::from_str(&env, "Fighter C"),
            &String::from_str(&env, "Fighter D"),
            &oracle,
            &(now + 1000),
            &(now + 2000),
        );

        let active_before = client.list_active_markets();
        assert_eq!(active_before.len(), 2);

        // Advance the ledger past the first market's end_time.
        env.ledger().with_mut(|l| l.timestamp = now + 500);

        let active_after = client.list_active_markets();
        assert_eq!(active_after.len(), 1);
        assert_eq!(active_after.get(0).unwrap().market_id, later_ending);
        assert_ne!(active_after.get(0).unwrap().market_id, soon_ending);
    }

    // ── C-07: set_admin ──────────────────────────────────────────────────────

    #[test]
    fn set_admin_transfers_admin_rights() {
        let (env, client, admin, treasury) = setup();
        init(&env, &client, &admin, &treasury);

        let new_admin = Address::generate(&env);
        client.set_admin(&admin, &new_admin);

        assert_eq!(client.get_admin(), new_admin);
    }

    #[test]
    fn set_admin_rejects_non_admin_caller() {
        let (env, client, admin, treasury) = setup();
        init(&env, &client, &admin, &treasury);

        let impostor = Address::generate(&env);
        let new_admin = Address::generate(&env);

        let result = client.try_set_admin(&impostor, &new_admin);
        assert_eq!(result, Err(Ok(ContractError::Unauthorized)));
        assert_eq!(client.get_admin(), admin);
    }

    // ── C-08: pause_factory / unpause_factory ───────────────────────────────

    #[test]
    fn pause_and_unpause_factory() {
        let (env, client, admin, treasury) = setup();
        init(&env, &client, &admin, &treasury);

        assert!(!client.is_paused());
        client.pause_factory(&admin);
        assert!(client.is_paused());
        client.unpause_factory(&admin);
        assert!(!client.is_paused());
    }

    #[test]
    fn create_market_reverts_while_paused() {
        let (env, client, admin, treasury) = setup();
        init(&env, &client, &admin, &treasury);

        client.pause_factory(&admin);

        let caller = Address::generate(&env);
        let oracle = Address::generate(&env);
        let now = env.ledger().timestamp();

        let result = client.try_create_market(
            &caller,
            &String::from_str(&env, "Fighter A"),
            &String::from_str(&env, "Fighter B"),
            &oracle,
            &(now + 100),
            &(now + 200),
        );
        assert_eq!(result, Err(Ok(ContractError::FactoryPaused)));
    }

    #[test]
    fn existing_markets_unaffected_by_pause() {
        let (env, client, admin, treasury) = setup();
        init(&env, &client, &admin, &treasury);

        let market_id = create_default_market(&env, &client);
        client.pause_factory(&admin);

        // Pausing the factory only blocks new creations; previously created
        // markets remain readable.
        assert!(client.get_market(&market_id).is_some());
    }

    #[test]
    fn pause_factory_rejects_non_admin_caller() {
        let (env, client, admin, treasury) = setup();
        init(&env, &client, &admin, &treasury);

        let impostor = Address::generate(&env);
        let result = client.try_pause_factory(&impostor);
        assert_eq!(result, Err(Ok(ContractError::Unauthorized)));
        assert!(!client.is_paused());
    }
}
