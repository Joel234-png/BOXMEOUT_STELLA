use soroban_sdk::{
    testutils::Address as _, token, xdr::ContractEventBody, Address, Env, Symbol, TryFromVal, Val, Vec,
};

/// Returns a default test environment with no special configuration.
pub fn create_test_env() -> Env {
    Env::default()
}

/// Generates a random test address suitable for use in unit tests.
pub fn create_test_address(env: &Env) -> Address {
    Address::generate(env)
}

/// Mints `amount` stroops of a freshly-registered token to `addr`.
/// Returns the token contract address so callers can set up the token client.
pub fn fund_address(env: &Env, addr: &Address, amount: i128) -> Address {
    let admin = Address::generate(env);
    let token_id = env.register_stellar_asset_contract_v2(admin).address();
    token::StellarAssetClient::new(env, &token_id).mint(addr, &amount);
    token_id
}

/// Returns the number of contract events recorded by the last invocation.
pub fn event_count(env: &Env) -> u32 {
    use soroban_sdk::testutils::Events as _;
    env.events().all().events().len() as u32
}

/// Returns `(topics, data)` of the event at `idx`, decoded back into host
/// values so they can be fed straight into the `shared::event_parser` functions.
pub fn event_at(env: &Env, idx: u32) -> (Vec<Val>, Val) {
    use soroban_sdk::testutils::Events as _;
    let all = env.events().all();
    let event = all.events().get(idx as usize).expect("event index out of range");
    let ContractEventBody::V0(body) = &event.body;
    let mut topics = Vec::new(env);
    for topic in body.topics.iter() {
        topics.push_back(Val::try_from_val(env, topic).expect("invalid topic"));
    }
    let data = Val::try_from_val(env, &body.data).expect("invalid data");
    (topics, data)
}

/// Returns `(topics, data)` of the most recently recorded event.
pub fn last_event(env: &Env) -> (Vec<Val>, Val) {
    let count = event_count(env);
    assert!(count > 0, "no events recorded");
    event_at(env, count - 1)
}

/// Returns the first topic of the most recent event as a `Symbol`.
pub fn last_event_name(env: &Env) -> Symbol {
    let (topics, _) = last_event(env);
    Symbol::try_from_val(env, &topics.get(0).expect("event has no topics")).expect("topic 0 is not a Symbol")
}
