//! Separate, TESTNET-ONLY Tidepool maker quote authorization.
//!
//! Distinct from the deployed official-1Click x3-guard; no migration, key setter,
//! upgrade path or mainnet deployment authority. The owner signs NEAR policy
//! transactions; this contract does not sign/fund EVM deposits or custody tokens.
//! Tidepool uses a trusted centralized testnet relayer. An external wallet can
//! bypass this advisory gate; the user's EVM wallet MUST separately enforce it.
use near_sdk::json_types::U128;
use near_sdk::serde::Deserialize;
use near_sdk::serde_json::{self};
use near_sdk::store::LookupMap;
use near_sdk::{bs58, env, near, require, AccountId, NearToken, PanicOnDefault, Promise};

pub const TIDEPOOL_MAKER: &str = "ed25519:U1Mu4u1nHnHmgABpCkfDGXt2FZj8k29UGqhkMAu4iWY";
pub const SOURCE_ASSET: &str = "nep141:base-0x38d81431b9f76d9699af7517040d5221d9dea252.tp-omft.testnet";
pub const DEST_ASSET: &str = "nep141:usdc.tidepool-sandbox.testnet";
pub const DEST_ACCOUNT: &str = "x3lang.testnet";
const MAX_PAYLOAD: usize = 8192;
const MAX_QUOTE_AGE_MS: u64 = 300_000;
const MAX_FUTURE_SKEW_MS: u64 = 30_000;
const BPS: u128 = 10_000;

#[near(serializers = [borsh, json])]
#[derive(Clone)]
pub struct IntentPolicy {
    pub program_sha256: String,
    pub amount_in: U128,
    pub minimum_out: U128,
    pub refund_to: String,
    pub expiry_ms: u64,
    pub max_slippage_bps: u32,
}

#[near(serializers = [borsh, json])]
#[derive(Clone)]
pub struct Authorization {
    pub owner: AccountId,
    pub nonce: String,
    pub policy: IntentPolicy,
    pub quote_sha256: Option<String>,
    pub revoked: bool,
}

#[derive(Deserialize)]
#[serde(crate = "near_sdk::serde", rename_all = "camelCase")]
struct QuoteRequest {
    dry: bool,
    swap_type: String,
    slippage_tolerance: u32,
    origin_asset: String,
    deposit_type: String,
    destination_asset: String,
    amount: String,
    refund_to: String,
    refund_type: String,
    recipient: String,
    recipient_type: String,
    deadline: String,
}
#[derive(Deserialize)]
#[serde(crate = "near_sdk::serde", rename_all = "camelCase")]
struct Quote {
    amount_in: String,
    amount_out: String,
    min_amount_out: String,
    deposit_address: Option<String>,
    deadline: Option<String>,
}
#[derive(Deserialize)]
#[serde(crate = "near_sdk::serde", rename_all = "camelCase")]
struct SignedResponse {
    timestamp: String,
    #[serde(rename = "quoteRequest")]
    request: QuoteRequest,
    quote: Quote,
}

#[near(contract_state)]
#[derive(PanicOnDefault)]
pub struct Contract {
    policies: LookupMap<String, Authorization>,
}

fn hex64(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
}
fn hex_address(s: &str) -> bool {
    s.len() == 42 && s.starts_with("0x") && s[2..].bytes().all(|b| b.is_ascii_hexdigit())
}
fn key(owner: &AccountId, nonce: &str) -> String {
    format!("{owner}:{nonce}")
}
fn charge_storage(start_bytes: u64) {
    let bytes = env::storage_usage().saturating_sub(start_bytes);
    let due = env::storage_byte_cost().saturating_mul(bytes as u128);
    let paid = env::attached_deposit();
    require!(
        paid >= due,
        format!("insufficient storage deposit: {} yoctoNEAR", due.as_yoctonear())
    );
    let change = paid.saturating_sub(due);
    if change > NearToken::from_yoctonear(0) {
        Promise::new(env::predecessor_account_id()).transfer(change).detach();
    }
}
fn read_amount(field: &str, s: &str) -> u128 {
    require!(
        !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()),
        format!("invalid {field}")
    );
    s.parse::<u128>()
        .unwrap_or_else(|_| env::panic_str(&format!("overflow in {field}")))
}
fn parse_utc_ms(s: &str) -> u64 {
    // Strict date format: YYYY-MM-DDTHH:MM:SS(.sss)Z.
    fn fail() -> ! {
        env::panic_str("timestamp must be strict UTC ISO-8601")
    }
    let body = s.strip_suffix('Z').unwrap_or_else(|| fail());
    let (main, frac) = body.split_once('.').unwrap_or((body, ""));
    let b = main.as_bytes();
    if b.len() != 19
        || b[4] != b'-'
        || b[7] != b'-'
        || b[10] != b'T'
        || b[13] != b':'
        || b[16] != b':'
        || !frac.bytes().all(|x| x.is_ascii_digit())
    {
        fail()
    }
    let digits = |a: usize, z: usize| -> u64 { main[a..z].parse::<u64>().unwrap_or_else(|_| fail()) };
    let (y, mo, d, h, mi, sec) = (
        digits(0, 4),
        digits(5, 7),
        digits(8, 10),
        digits(11, 13),
        digits(14, 16),
        digits(17, 19),
    );
    let leap = y % 4 == 0 && (y % 100 != 0 || y % 400 == 0);
    let days_in_month = [31, if leap { 29 } else { 28 }, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
    if !(1970..=9999).contains(&y)
        || !(1..=12).contains(&mo)
        || d == 0
        || d > days_in_month[(mo - 1) as usize]
        || h > 23
        || mi > 59
        || sec > 59
    {
        fail()
    }
    let (yy, mm) = if mo <= 2 { (y - 1, mo + 9) } else { (y, mo - 3) };
    let era = yy / 400;
    let yoe = yy - era * 400;
    let doy = (153 * mm + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let ms = format!("{:0<3}", &frac[..frac.len().min(3)])
        .parse::<u64>()
        .unwrap_or(0);
    ((era * 146097 + doe - 719468) * 86400 + h * 3600 + mi * 60 + sec) * 1000 + ms
}

#[near]
impl Contract {
    #[init]
    pub fn new() -> Self {
        // Avoid deploying this contract under mainnet network domains.
        require!(
            env::current_account_id().as_str().ends_with(".testnet"),
            "testnet deployment only"
        );
        Self {
            policies: LookupMap::new(b"x3-tidepool-policies-v1".as_slice()),
        }
    }
    #[payable]
    pub fn register(&mut self, nonce: String, policy: IntentPolicy) {
        require!(
            (8..=128).contains(&nonce.len())
                && nonce
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_'),
            "invalid nonce"
        );
        require!(hex64(&policy.program_sha256), "program digest required");
        require!(
            policy.amount_in.0 > 0 && policy.minimum_out.0 > 0 && policy.max_slippage_bps <= 100,
            "unsafe spending policy"
        );
        require!(hex_address(&policy.refund_to), "explicit EVM refund address required");
        let now = env::block_timestamp_ms();
        require!(
            policy.expiry_ms > now + 60_000 && policy.expiry_ms <= now + 3_600_000,
            "policy expiry out of range"
        );
        let initial_storage = env::storage_usage();
        let owner = env::predecessor_account_id();
        let id = key(&owner, &nonce);
        require!(!self.policies.contains_key(&id), "nonce already registered");
        self.policies.insert(
            id,
            Authorization {
                owner,
                nonce,
                policy,
                quote_sha256: None,
                revoked: false,
            },
        );
        self.policies.flush();
        charge_storage(initial_storage);
    }
    pub fn get_policy(&self, owner: AccountId, nonce: String) -> Option<Authorization> {
        self.policies.get(&key(&owner, &nonce)).cloned()
    }
    pub fn revoke(&mut self, nonce: String) {
        let owner = env::predecessor_account_id();
        let id = key(&owner, &nonce);
        let mut record = self
            .policies
            .get(&id)
            .cloned()
            .unwrap_or_else(|| env::panic_str("unknown owner nonce"));
        require!(!record.revoked, "already revoked");
        record.revoked = true;
        self.policies.insert(id, record);
        self.policies.flush();
    }
    #[payable]
    pub fn authorize(&mut self, nonce: String, signed_payload: String, signature: String) -> String {
        require!(
            !signed_payload.is_empty() && signed_payload.len() <= MAX_PAYLOAD,
            "bad quote size"
        );
        require!(
            !signed_payload.contains("\"signature\""),
            "signature must be separate from signed body"
        );
        let initial_storage = env::storage_usage();
        let owner = env::predecessor_account_id();
        let id = key(&owner, &nonce);
        let mut record = self
            .policies
            .get(&id)
            .cloned()
            .unwrap_or_else(|| env::panic_str("no owner policy for nonce"));
        require!(!record.revoked, "owner revoked nonce");
        require!(record.quote_sha256.is_none(), "nonce was already authorized");
        let now = env::block_timestamp_ms();
        require!(now < record.policy.expiry_ms, "policy expired");
        // No JSON parsing before signature authentication. The caller supplies
        // compact JSON bytes of the provider response with signature removed.
        let pk: Vec<u8> = bs58::decode(TIDEPOOL_MAKER.trim_start_matches("ed25519:"))
            .into_vec()
            .unwrap_or_else(|_| env::panic_str("invalid pinned key"));
        require!(pk.len() == 32, "bad maker key");
        let mut key = [0u8; 32];
        key.copy_from_slice(&pk);
        let raw: Vec<u8> = bs58::decode(signature.strip_prefix("ed25519:").unwrap_or(""))
            .into_vec()
            .unwrap_or_default();
        require!(raw.len() == 64, "invalid signature bytes");
        let mut sig = [0u8; 64];
        sig.copy_from_slice(&raw);
        let digest = env::sha256(signed_payload.as_bytes());
        require!(
            env::ed25519_verify(&sig, &digest, &key),
            "unauthenticated testnet maker quote"
        );
        let response: SignedResponse =
            serde_json::from_str(&signed_payload).unwrap_or_else(|_| env::panic_str("malformed signed response"));
        let r = response.request;
        let q = response.quote;
        require!(
            !r.dry
                && r.swap_type == "EXACT_INPUT"
                && r.deposit_type == "ORIGIN_CHAIN"
                && r.refund_type == "ORIGIN_CHAIN"
                && r.recipient_type == "DESTINATION_CHAIN",
            "unsupported quote type"
        );
        require!(
            r.origin_asset == SOURCE_ASSET && r.destination_asset == DEST_ASSET && r.recipient == DEST_ACCOUNT,
            "chain/asset/recipient mismatch"
        );
        require!(
            r.refund_to.to_lowercase() == record.policy.refund_to.to_lowercase(),
            "refund mismatch"
        );
        require!(
            r.slippage_tolerance <= record.policy.max_slippage_bps,
            "slippage limit exceeded"
        );
        require!(
            read_amount("request amount", &r.amount) == record.policy.amount_in.0
                && read_amount("amountIn", &q.amount_in) == record.policy.amount_in.0,
            "spend mismatch"
        );
        let amount_out = read_amount("amountOut", &q.amount_out);
        let min_out = read_amount("minAmountOut", &q.min_amount_out);
        require!(
            amount_out > 0 && min_out >= record.policy.minimum_out.0 && min_out <= amount_out,
            "minimum return violated"
        );
        let left = min_out
            .checked_mul(BPS)
            .unwrap_or_else(|| env::panic_str("slippage multiplication overflow"));
        let right = amount_out
            .checked_mul(BPS - record.policy.max_slippage_bps as u128)
            .unwrap_or_else(|| env::panic_str("slippage multiplication overflow"));
        require!(left >= right, "quote slippage exceeded");
        let deposit_address = q.deposit_address.as_deref().unwrap_or("");
        require!(hex_address(deposit_address), "origin EVM deposit address invalid");
        let issue_ms = parse_utc_ms(&response.timestamp);
        let deadline_ms = parse_utc_ms(q.deadline.as_deref().unwrap_or(""));
        let requested_deadline = parse_utc_ms(&r.deadline);
        require!(
            issue_ms <= now + MAX_FUTURE_SKEW_MS && now <= issue_ms.saturating_add(MAX_QUOTE_AGE_MS),
            "stale or future quote"
        );
        require!(
            deadline_ms > now + 60_000 && deadline_ms <= record.policy.expiry_ms && requested_deadline >= deadline_ms,
            "quote deadline unsafe"
        );
        let sha = env::sha256(signed_payload.as_bytes());
        let hash = sha.iter().map(|b| format!("{b:02x}")).collect::<String>();
        record.quote_sha256 = Some(hash.clone());
        self.policies.insert(id, record);
        self.policies.flush();
        charge_storage(initial_storage);
        env::log_str(&format!(
            "X3_TESTNET_QUOTE_AUTHORIZED owner={owner} nonce={nonce} sha256={hash} (no deposit or settlement)"
        ));
        hash
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use near_sdk::test_utils::VMContextBuilder;
    use near_sdk::testing_env;

    fn context(predecessor: &str, deposit: NearToken) {
        let mut ctx = VMContextBuilder::new();
        ctx.current_account_id("x3-tidepool-guard.testnet".parse().unwrap())
            .predecessor_account_id(predecessor.parse().unwrap())
            .block_timestamp(1_762_500_000_000_000_000)
            .attached_deposit(deposit);
        testing_env!(ctx.build());
    }
    fn sample_policy() -> IntentPolicy {
        IntentPolicy {
            program_sha256: "a".repeat(64),
            amount_in: U128(1_000_000),
            minimum_out: U128(980_000),
            refund_to: "0x1234567890123456789012345678901234567890".into(),
            expiry_ms: 1_762_500_600_000,
            max_slippage_bps: 100,
        }
    }
    #[test]
    fn owner_policy_can_be_registered_and_revoked() {
        context("x3lang.testnet", NearToken::from_near(1));
        let mut contract = Contract::new();
        let nonce = "demo_0001".to_string();
        contract.register(nonce.clone(), sample_policy());
        let saved = contract
            .get_policy("x3lang.testnet".parse().unwrap(), nonce.clone())
            .unwrap();
        assert_eq!(saved.policy.amount_in.0, 1_000_000);
        assert!(!saved.revoked);
        contract.revoke(nonce.clone());
        assert!(
            contract
                .get_policy("x3lang.testnet".parse().unwrap(), nonce)
                .unwrap()
                .revoked
        );
    }
    #[test]
    #[should_panic(expected = "insufficient storage deposit")]
    fn storage_underfunding_must_fail_closed() {
        context("x3lang.testnet", NearToken::from_yoctonear(0));
        let mut contract = Contract::new();
        contract.register("demo_0001".into(), sample_policy());
    }
    #[test]
    #[should_panic(expected = "nonce already registered")]
    fn cannot_reuse_nonce() {
        context("x3lang.testnet", NearToken::from_near(1));
        let mut contract = Contract::new();
        contract.register("demo_0001".into(), sample_policy());
        contract.register("demo_0001".into(), sample_policy());
    }
    #[test]
    #[should_panic(expected = "no owner policy for nonce")]
    fn other_owner_cannot_authorize_policy() {
        context("x3lang.testnet", NearToken::from_near(1));
        let mut contract = Contract::new();
        contract.register("demo_0001".into(), sample_policy());
        context("attacker.testnet", NearToken::from_near(1));
        contract.authorize("demo_0001".into(), "{}".into(), "ed25519:bad".into());
    }
    #[test]
    #[should_panic(expected = "invalid signature bytes")]
    fn malformed_signature_must_fail_before_quote_parsing() {
        context("x3lang.testnet", NearToken::from_near(1));
        let mut contract = Contract::new();
        contract.register("demo_0001".into(), sample_policy());
        contract.authorize("demo_0001".into(), "{}".into(), "ed25519:bad".into());
    }
    #[test]
    #[should_panic(expected = "owner revoked nonce")]
    fn revoked_policy_must_not_authorize() {
        context("x3lang.testnet", NearToken::from_near(1));
        let mut contract = Contract::new();
        contract.register("demo_0001".into(), sample_policy());
        contract.revoke("demo_0001".into());
        contract.authorize("demo_0001".into(), "{}".into(), "ed25519:bad".into());
    }
}

#[cfg(test)]
mod authentic_signature_tests {
    use super::*;
    use near_sdk::test_utils::VMContextBuilder;
    use near_sdk::testing_env;
    #[test]
    #[should_panic(expected = "unsupported quote type")]
    fn authentic_tidepool_signature_passes_crypto_but_dry_quote_cannot_authorize() {
        let fixture: serde_json::Value = serde_json::from_str(include_str!("signed_dry_fixture.json")).unwrap();
        let body = fixture["signed_payload"].as_str().unwrap().to_string();
        let sig = fixture["signature"].as_str().unwrap().to_string();
        let response: SignedResponse = serde_json::from_str(&body).unwrap();
        let t = parse_utc_ms(&response.timestamp);
        let mut ctx = VMContextBuilder::new();
        ctx.current_account_id("x3-tidepool-guard.testnet".parse().unwrap())
            .predecessor_account_id("x3lang.testnet".parse().unwrap())
            .block_timestamp(t * 1_000_000)
            .attached_deposit(NearToken::from_near(1));
        testing_env!(ctx.build());
        let mut contract = Contract::new();
        contract.register(
            "demo_0001".into(),
            IntentPolicy {
                program_sha256: "a".repeat(64),
                amount_in: U128(1_000_000),
                minimum_out: U128(980_000),
                refund_to: "0x0000000000000000000000000000000000000001".into(),
                expiry_ms: t + 600_000,
                max_slippage_bps: 100,
            },
        );
        // Must verify authentic provider signature BEFORE rejecting the dry quote.
        contract.authorize("demo_0001".into(), body, sig);
    }
}

#[cfg(test)]
mod authentic_live_tests {
    use super::*;
    use near_sdk::test_utils::VMContextBuilder;
    use near_sdk::testing_env;

    fn setup(refund: &str) -> (Contract, String, String, String) {
        let fixture: serde_json::Value = serde_json::from_str(include_str!("signed_live_fixture.json")).unwrap();
        assert_eq!(
            fixture["classification"],
            "REAL_SIGNED_LIVE_QUOTE_NO_DEPOSIT_NO_GUARD_AUTHORIZATION"
        );
        let body = fixture["signed_payload"].as_str().unwrap().to_string();
        let sig = fixture["signature"].as_str().unwrap().to_string();
        let quote: SignedResponse = serde_json::from_str(&body).unwrap();
        let issued = parse_utc_ms(&quote.timestamp);
        let mut ctx = VMContextBuilder::new();
        ctx.current_account_id("x3-tidepool-guard.testnet".parse().unwrap())
            .predecessor_account_id("x3lang.testnet".parse().unwrap())
            .block_timestamp(issued * 1_000_000)
            .attached_deposit(NearToken::from_near(1));
        testing_env!(ctx.build());
        let mut contract = Contract::new();
        let nonce = "realquote0001".to_string();
        contract.register(
            nonce.clone(),
            IntentPolicy {
                program_sha256: "b".repeat(64),
                amount_in: U128(1_000_000),
                minimum_out: U128(980_000),
                refund_to: refund.to_string(),
                expiry_ms: issued + 45 * 60 * 1000,
                max_slippage_bps: 100,
            },
        );
        (contract, nonce, body, sig)
    }

    #[test]
    fn authentic_live_quote_authorizes_exact_owner_policy() {
        let (mut contract, nonce, body, sig) = setup("0x5aF0c1BF95B3A158f6Fc48e3DED35903b2fa0721");
        let hash = contract.authorize(nonce.clone(), body, sig);
        assert_eq!(hash.len(), 64);
        let record = contract.get_policy("x3lang.testnet".parse().unwrap(), nonce).unwrap();
        assert_eq!(record.quote_sha256, Some(hash));
    }
    #[test]
    #[should_panic(expected = "nonce was already authorized")]
    fn authentic_live_quote_cannot_reuse_nonce() {
        let (mut contract, nonce, body, sig) = setup("0x5aF0c1BF95B3A158f6Fc48e3DED35903b2fa0721");
        contract.authorize(nonce.clone(), body.clone(), sig.clone());
        contract.authorize(nonce, body, sig);
    }
    #[test]
    #[should_panic(expected = "unauthenticated testnet maker quote")]
    fn authentic_live_quote_recipient_tamper_rejected_by_crypto() {
        let (mut contract, nonce, body, sig) = setup("0x5aF0c1BF95B3A158f6Fc48e3DED35903b2fa0721");
        let mut changed = body.replace("x3lang.testnet", "attacker.testnet");
        assert_ne!(changed, body);
        contract.authorize(nonce, std::mem::take(&mut changed), sig);
    }
    #[test]
    #[should_panic(expected = "refund mismatch")]
    fn authentic_live_quote_cannot_override_owner_refund_address() {
        let (mut contract, nonce, body, sig) = setup("0x1234567890123456789012345678901234567890");
        contract.authorize(nonce, body, sig);
    }
}
