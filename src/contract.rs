use crate::metadata::{self, TokenMetadata};
use crate::storage_types::DataKey;
use crate::{admin, balance};
use soroban_sdk::{contract, contractimpl, Address, Env, String};

#[contract]
pub struct VeriTixPay;

/// Stores the admin and the token metadata, refusing a second call.
fn initialize_state(e: &Env, admin_addr: &Address, meta: &TokenMetadata) {
    if admin::is_initialized(e) {
        panic!("AlreadyInitialized: contract state is locked");
    }
    admin_addr.require_auth();
    e.storage().persistent().set(&DataKey::Admin, admin_addr);
    e.storage()
        .persistent()
        .set(&DataKey::InitializedAtLedger, &e.ledger().sequence());
    metadata::store(e, meta);
}

#[contractimpl]
impl VeriTixPay {
    /// Sets the admin and the default token metadata.
    ///
    /// The admin address is the root of trust for every privileged entry point,
    /// so initialization is deliberately one-shot: a second call is rejected
    /// rather than silently replacing the admin.
    pub fn initialize(e: Env, admin_addr: Address) {
        initialize_state(&e, &admin_addr, &metadata::defaults(&e));
    }

    /// Sets the admin and caller-supplied token metadata.
    ///
    /// Same one-shot rule as [`initialize`]; use this when the deployment needs
    /// a token name, symbol or precision other than the defaults.
    pub fn initialize_with_metadata(
        e: Env,
        admin_addr: Address,
        name: String,
        symbol: String,
        decimals: u32,
    ) {
        initialize_state(
            &e,
            &admin_addr,
            &TokenMetadata {
                name,
                symbol,
                decimals,
            },
        );
    }

    /// True once an admin has been stored.
    pub fn is_initialized(e: Env) -> bool {
        admin::is_initialized(&e)
    }

    /// The address currently holding admin authority.
    pub fn admin(e: Env) -> Address {
        admin::admin(&e)
    }

    /// Ledger on which the contract was initialized, 0 when it never was.
    pub fn initialized_at_ledger(e: Env) -> u32 {
        admin::initialized_at_ledger(&e)
    }

    /// The token name.
    pub fn name(e: Env) -> String {
        metadata::load(&e).name
    }

    /// The token symbol.
    pub fn symbol(e: Env) -> String {
        metadata::load(&e).symbol
    }

    /// The number of decimals the token is scaled by.
    pub fn decimals(e: Env) -> u32 {
        metadata::load(&e).decimals
    }

    /// Tokens held by `account`; 0 for an address that has never been credited.
    pub fn balance(e: Env, account: Address) -> i128 {
        balance::balance_of(&e, &account)
    }

    /// Tokens in circulation.
    pub fn total_supply(e: Env) -> i128 {
        balance::total_supply(&e)
    }

    /// Mints `amount` new tokens to `to`.
    ///
    /// Admin-only and supply-capped: the cap is the first place a supply limit
    /// has to hold, so the check lives in the balance module next to the
    /// arithmetic it protects rather than being repeated by every caller.
    pub fn mint(e: Env, admin_addr: Address, to: Address, amount: i128) {
        admin::check_admin(&e, &admin_addr);
        balance::mint(&e, &to, amount);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::testutils::{Address as _, Events as _};
    use soroban_sdk::{symbol_short, vec, xdr, FromVal};

    fn setup() -> (Env, VeriTixPayClient<'static>, Address) {
        let e = Env::default();
        e.mock_all_auths();
        let contract_id = e.register_contract(None, VeriTixPay);
        let client = VeriTixPayClient::new(&e, &contract_id);
        let admin = Address::generate(&e);
        client.initialize(&admin);
        (e, client, admin)
    }

    /// Installs a supply cap. Choosing the cap belongs to the initializer, so
    /// the test writes the storage key directly instead of adding a setter that
    /// no issue has asked for.
    fn install_cap(e: &Env, contract_id: &Address, cap: i128) {
        e.as_contract(contract_id, || {
            e.storage().persistent().set(&DataKey::MaxSupply, &cap);
        });
    }

    fn setup_capped(cap: i128) -> (Env, VeriTixPayClient<'static>, Address) {
        let e = Env::default();
        e.mock_all_auths();
        let contract_id = e.register_contract(None, VeriTixPay);
        let client = VeriTixPayClient::new(&e, &contract_id);
        let admin = Address::generate(&e);
        client.initialize(&admin);
        install_cap(&e, &contract_id, cap);
        (e, client, admin)
    }

    #[test]
    fn test_initialize_stores_admin_and_defaults() {
        let (e, client, admin) = setup();
        assert!(client.is_initialized());
        assert_eq!(client.admin(), admin);
        assert_eq!(client.initialized_at_ledger(), e.ledger().sequence());
        assert_eq!(client.name(), String::from_str(&e, "VeriTix"));
        assert_eq!(client.symbol(), String::from_str(&e, "VTX"));
        assert_eq!(client.decimals(), 7);
    }

    #[test]
    #[should_panic(expected = "AlreadyInitialized")]
    fn test_initialize_twice_panics() {
        let (e, client, _admin) = setup();
        let other = Address::generate(&e);
        client.initialize(&other);
    }

    #[test]
    fn test_initialize_with_metadata_overrides_defaults() {
        let e = Env::default();
        e.mock_all_auths();
        let contract_id = e.register_contract(None, VeriTixPay);
        let client = VeriTixPayClient::new(&e, &contract_id);
        let admin = Address::generate(&e);

        client.initialize_with_metadata(
            &admin,
            &String::from_str(&e, "Event Tickets"),
            &String::from_str(&e, "VTIX"),
            &2,
        );

        assert_eq!(client.name(), String::from_str(&e, "Event Tickets"));
        assert_eq!(client.symbol(), String::from_str(&e, "VTIX"));
        assert_eq!(client.decimals(), 2);
    }

    #[test]
    fn test_balance_and_supply_start_at_zero() {
        let (e, client, _admin) = setup();
        let user = Address::generate(&e);
        assert_eq!(client.balance(&user), 0);
        assert_eq!(client.total_supply(), 0);
    }

    #[test]
    fn test_mint_credits_balance_and_grows_supply() {
        let (e, client, admin) = setup();
        let user = Address::generate(&e);

        client.mint(&admin, &user, &1_000);

        assert_eq!(client.balance(&user), 1_000);
        assert_eq!(client.total_supply(), 1_000);
    }

    #[test]
    fn test_mint_accumulates_across_calls() {
        let (e, client, admin) = setup();
        let user = Address::generate(&e);

        client.mint(&admin, &user, &1_000);
        client.mint(&admin, &user, &500);

        assert_eq!(client.balance(&user), 1_500);
        assert_eq!(client.total_supply(), 1_500);
    }

    #[test]
    #[should_panic(expected = "Unauthorized: caller is not the contract admin")]
    fn test_mint_rejects_non_admin() {
        let (e, client, _admin) = setup();
        let stranger = Address::generate(&e);
        let user = Address::generate(&e);
        client.mint(&stranger, &user, &1_000);
    }

    #[test]
    #[should_panic(expected = "InvalidAmount")]
    fn test_mint_rejects_zero_amount() {
        let (e, client, admin) = setup();
        let user = Address::generate(&e);
        client.mint(&admin, &user, &0);
    }

    #[test]
    #[should_panic(expected = "InvalidAmount")]
    fn test_mint_rejects_negative_amount() {
        let (e, client, admin) = setup();
        let user = Address::generate(&e);
        client.mint(&admin, &user, &(-1));
    }

    #[test]
    #[should_panic(expected = "SupplyCapExceeded")]
    fn test_mint_rejects_amount_over_the_cap() {
        let (e, client, admin) = setup_capped(1_000);
        let user = Address::generate(&e);
        client.mint(&admin, &user, &1_001);
    }

    #[test]
    fn test_mint_allows_amount_exactly_at_the_cap() {
        let (e, client, admin) = setup_capped(1_000);
        let user = Address::generate(&e);
        client.mint(&admin, &user, &1_000);
        assert_eq!(client.total_supply(), 1_000);
    }

    #[test]
    #[should_panic(expected = "SupplyCapExceeded")]
    fn test_mint_counts_prior_mints_against_the_cap() {
        let (e, client, admin) = setup_capped(1_000);
        let user = Address::generate(&e);
        client.mint(&admin, &user, &600);
        client.mint(&admin, &user, &500);
    }

    #[test]
    fn test_mint_emits_event_with_standard_topics_and_data() {
        let (e, client, admin) = setup();
        let user = Address::generate(&e);

        client.mint(&admin, &user, &1_000);

        let events = e.events().all();
        assert_eq!(events.events().len(), 1);
        let xdr::ContractEventBody::V0(body) = &events.events()[0].body else {
            panic!("expected a v0 contract event");
        };
        assert_eq!(
            body.topics,
            std::vec![
                xdr::ScVal::from_val(&e, &symbol_short!("mint").to_val()),
                xdr::ScVal::from_val(&e, &user.to_val()),
            ]
        );
        assert_eq!(
            body.data,
            xdr::ScVal::from_val(&e, &vec![&e, 1_000i128].to_val())
        );
    }
}
