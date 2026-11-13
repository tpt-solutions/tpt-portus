// SPDX-License-Identifier: MIT OR Apache-2.0
use tpt_portus::config::{ConfigError, PortusConfig};

#[test]
fn shipped_portus_toml_matches_defaults() {
    let text = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/portus.toml"))
        .expect("portus.toml exists");
    let parsed = PortusConfig::from_toml_str(&text).expect("portus.toml parses");
    assert_eq!(parsed, PortusConfig::default());
}

#[test]
fn missing_keys_fall_back_to_defaults() {
    let cfg = PortusConfig::from_toml_str("[portus]\nnode_id = \"n2\"\n").unwrap();
    assert_eq!(cfg.portus.node_id, "n2");
    assert_eq!(cfg.portus.relayer.min_acknowledgments, 3);
}

#[test]
fn env_overrides_apply_by_dotted_path() {
    let vars = vec![
        ("PORTUS_NODE_ID".to_string(), "node-x".to_string()),
        (
            "PORTUS_STATERA_ENDPOINT".to_string(),
            "grpc://s:1".to_string(),
        ),
        (
            "PORTUS_RELAYER_RATE_LIMIT_RPS".to_string(),
            "25".to_string(),
        ),
        (
            "PORTUS_CHAINS_BITCOIN_RPC_URL".to_string(),
            "http://btc".to_string(),
        ),
        ("PORTUS_NOT_A_KEY".to_string(), "ignored".to_string()),
        ("UNRELATED".to_string(), "ignored".to_string()),
    ];
    let cfg = PortusConfig::default().apply_env_overrides(vars).unwrap();
    assert_eq!(cfg.portus.node_id, "node-x");
    assert_eq!(cfg.portus.statera.endpoint, "grpc://s:1");
    assert_eq!(cfg.portus.relayer.rate_limit_rps, 25);
    assert_eq!(cfg.portus.chains.bitcoin.rpc_url, "http://btc");
}

#[test]
fn bad_integer_override_is_rejected() {
    let vars = vec![(
        "PORTUS_RELAYER_RATE_LIMIT_RPS".to_string(),
        "lots".to_string(),
    )];
    let err = PortusConfig::default()
        .apply_env_overrides(vars)
        .unwrap_err();
    assert!(matches!(err, ConfigError::Override { .. }));
}

#[test]
fn fee_split_must_sum_to_100() {
    let text = "[portus.relayer]\nfee_split_relayer_pct = 80\n";
    let err = PortusConfig::from_toml_str(text).unwrap_err();
    assert!(matches!(err, ConfigError::Invalid(_)));
}
