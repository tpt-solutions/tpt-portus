// SPDX-License-Identifier: MIT OR Apache-2.0
//! Configuration (DESIGN.md §7).
//!
//! Defaults mirror `portus.toml` at the repository root. Environment overrides
//! follow §7.2: any leaf key is overridable with `PORTUS_` plus its dotted path
//! upper-cased and joined with `_` (e.g. `PORTUS_STATERA_ENDPOINT`).
//!
//! Deviation from §7.1: the per-chain `chain_id` keys are not modelled. §4.2
//! states that the numeric ids in the v0.2 config were a genuine bug, and chain
//! identity is now the `ChainId` tagged enum in [`crate::types`].

use serde::{Deserialize, Serialize};
use thiserror::Error;
use toml::Value;

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("failed to parse portus.toml: {0}")]
    Parse(#[from] toml::de::Error),
    #[error("invalid value for environment override {var}: {reason}")]
    Override { var: String, reason: String },
    #[error("invalid configuration: {0}")]
    Invalid(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct PortusConfig {
    pub portus: PortusSection,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct PortusSection {
    pub node_id: String,
    pub listen_addr: String,
    pub log_level: String,
    pub statera: StateraConfig,
    pub relayer: RelayerConfig,
    pub chains: ChainsConfig,
    pub rollback: RollbackConfig,
    pub security: SecurityConfig,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct StateraConfig {
    pub endpoint: String,
    pub tls_cert: String,
    pub timeout_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct RelayerConfig {
    pub message_bus: String,
    pub min_acknowledgments: u32,
    pub claim_timeout_secs: u64,
    pub backup_pool_id: String,
    pub rate_limit_rps: u32,
    pub fee_split_relayer_pct: u8,
    pub fee_split_liquidity_pct: u8,
    pub fee_split_slash_reserve_pct: u8,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct ChainsConfig {
    pub ethereum: EvmChainConfig,
    pub solana: SolanaChainConfig,
    pub cosmos: CosmosChainConfig,
    pub bitcoin: BitcoinChainConfig,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct EvmChainConfig {
    pub rpc_url: String,
    pub finality_blocks: u64,
    pub endpoint_contract: String,
    pub light_client_sync_interval_secs: u64,
    pub dispute_window_secs: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct SolanaChainConfig {
    pub rpc_url: String,
    pub finality_slots: u64,
    pub endpoint_program: String,
    pub dispute_window_secs: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct CosmosChainConfig {
    pub rpc_url: String,
    pub finality_blocks: u64,
    pub ibc_client_id: String,
    pub dispute_window_secs: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct BitcoinChainConfig {
    pub rpc_url: String,
    pub confirmation_depth: u64,
    pub dispute_window_secs: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct RollbackConfig {
    pub check_interval_secs: u64,
    pub max_retry_attempts: u32,
    pub slash_percentage: u8,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct SecurityConfig {
    pub hsm_endpoint: String,
    pub key_id: String,
    pub statera_signature_scheme: String,
}

impl Default for PortusSection {
    fn default() -> Self {
        Self {
            node_id: "portus-node-01".into(),
            listen_addr: "0.0.0.0:9400".into(),
            log_level: "info".into(),
            statera: StateraConfig::default(),
            relayer: RelayerConfig::default(),
            chains: ChainsConfig::default(),
            rollback: RollbackConfig::default(),
            security: SecurityConfig::default(),
        }
    }
}

impl Default for StateraConfig {
    fn default() -> Self {
        Self {
            endpoint: "grpc://statera.internal:9300".into(),
            tls_cert: "/etc/tpt/certs/statera-ca.pem".into(),
            timeout_ms: 5000,
        }
    }
}

impl Default for RelayerConfig {
    fn default() -> Self {
        Self {
            message_bus: "nats://nats.internal:4222".into(),
            min_acknowledgments: 3,
            claim_timeout_secs: 30,
            backup_pool_id: "relayer-backup-01".into(),
            rate_limit_rps: 10,
            fee_split_relayer_pct: 70,
            fee_split_liquidity_pct: 20,
            fee_split_slash_reserve_pct: 10,
        }
    }
}

impl Default for EvmChainConfig {
    fn default() -> Self {
        Self {
            rpc_url: "https://eth-mainnet.internal:8545".into(),
            finality_blocks: 12,
            endpoint_contract: "0x...".into(),
            light_client_sync_interval_secs: 12,
            dispute_window_secs: 600,
        }
    }
}

impl Default for SolanaChainConfig {
    fn default() -> Self {
        Self {
            rpc_url: "https://solana-mainnet.internal:8899".into(),
            finality_slots: 32,
            endpoint_program: "PortusEndpoint1111111111111111111111".into(),
            dispute_window_secs: 5,
        }
    }
}

impl Default for CosmosChainConfig {
    fn default() -> Self {
        Self {
            rpc_url: "https://cosmos-mainnet.internal:26657".into(),
            finality_blocks: 1,
            ibc_client_id: "07-tendermint-42".into(),
            dispute_window_secs: 5,
        }
    }
}

impl Default for BitcoinChainConfig {
    fn default() -> Self {
        Self {
            rpc_url: "https://btc-mainnet.internal:8332".into(),
            confirmation_depth: 6,
            dispute_window_secs: 3600,
        }
    }
}

impl Default for RollbackConfig {
    fn default() -> Self {
        Self {
            check_interval_secs: 5,
            max_retry_attempts: 3,
            slash_percentage: 10,
        }
    }
}

impl Default for SecurityConfig {
    fn default() -> Self {
        Self {
            hsm_endpoint: "pkcs11:///dev/hsm0".into(),
            key_id: "portus-signing-key-01".into(),
            statera_signature_scheme: "threshold-ed25519".into(),
        }
    }
}

impl PortusConfig {
    /// Parse a `portus.toml` document. Missing keys fall back to the §7.1 defaults.
    pub fn from_toml_str(input: &str) -> Result<Self, ConfigError> {
        let config: Self = toml::from_str(input)?;
        config.validate()?;
        Ok(config)
    }

    /// Apply `PORTUS_`-prefixed overrides from `vars` (DESIGN.md §7.2).
    /// Variables with the prefix that match no configuration key are ignored.
    pub fn apply_env_overrides<I>(self, vars: I) -> Result<Self, ConfigError>
    where
        I: IntoIterator<Item = (String, String)>,
    {
        let mut tree =
            Value::try_from(&self).map_err(|e| ConfigError::Invalid(format!("serialize: {e}")))?;

        let mut leaves = Vec::new();
        collect_leaf_paths(&tree, &mut Vec::new(), &mut leaves);

        for (var, raw) in vars {
            if !var.starts_with("PORTUS_") {
                continue;
            }
            let Some(path) = leaves.iter().find(|p| env_name(p) == var) else {
                continue;
            };
            let slot = lookup_mut(&mut tree, path)
                .ok_or_else(|| ConfigError::Invalid(format!("missing key for {var}")))?;
            *slot = coerce_like(slot, &raw).map_err(|reason| ConfigError::Override {
                var: var.clone(),
                reason,
            })?;
        }

        let config: Self = tree
            .try_into()
            .map_err(|e| ConfigError::Invalid(format!("override produced invalid config: {e}")))?;
        config.validate()?;
        Ok(config)
    }

    /// Load defaults, then apply overrides from the process environment.
    pub fn from_env(base: Self) -> Result<Self, ConfigError> {
        base.apply_env_overrides(std::env::vars())
    }

    /// Cross-field checks that the TOML schema cannot express.
    pub fn validate(&self) -> Result<(), ConfigError> {
        let r = &self.portus.relayer;
        let total = u32::from(r.fee_split_relayer_pct)
            + u32::from(r.fee_split_liquidity_pct)
            + u32::from(r.fee_split_slash_reserve_pct);
        if total != 100 {
            return Err(ConfigError::Invalid(format!(
                "relayer fee split must sum to 100, got {total}"
            )));
        }
        if r.min_acknowledgments == 0 {
            return Err(ConfigError::Invalid(
                "relayer.min_acknowledgments must be at least 1".into(),
            ));
        }
        Ok(())
    }
}

/// Environment variable name for a leaf path: `["portus","statera","endpoint"]`
/// becomes `PORTUS_STATERA_ENDPOINT`.
fn env_name(path: &[String]) -> String {
    path.join("_").replace(['-', '.'], "_").to_ascii_uppercase()
}

fn collect_leaf_paths(v: &Value, path: &mut Vec<String>, out: &mut Vec<Vec<String>>) {
    match v {
        Value::Table(table) => {
            for (key, child) in table {
                path.push(key.clone());
                collect_leaf_paths(child, path, out);
                path.pop();
            }
        }
        _ => out.push(path.clone()),
    }
}

fn lookup_mut<'a>(v: &'a mut Value, path: &[String]) -> Option<&'a mut Value> {
    path.iter()
        .try_fold(v, |node, key| node.as_table_mut()?.get_mut(key))
}

/// Parse `raw` as the same TOML type as `existing`.
fn coerce_like(existing: &Value, raw: &str) -> Result<Value, String> {
    match existing {
        Value::String(_) => Ok(Value::String(raw.to_owned())),
        Value::Integer(_) => raw
            .parse::<i64>()
            .map(Value::Integer)
            .map_err(|e| format!("expected integer: {e}")),
        Value::Float(_) => raw
            .parse::<f64>()
            .map(Value::Float)
            .map_err(|e| format!("expected float: {e}")),
        Value::Boolean(_) => raw
            .parse::<bool>()
            .map(Value::Boolean)
            .map_err(|e| format!("expected boolean: {e}")),
        other => Err(format!("unsupported override type: {}", other.type_str())),
    }
}
