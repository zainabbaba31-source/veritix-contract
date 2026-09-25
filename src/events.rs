use soroban_sdk::{contractevent, Address};

/// New tokens were credited to `to`.
///
/// Topics are `["mint", to: Address]` and the data is `[amount: i128]`, the
/// shape indexers subscribe to for a supply increase.
#[contractevent(data_format = "vec")]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Mint {
    #[topic]
    pub to: Address,
    pub amount: i128,
}
