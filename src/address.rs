// SPDX-License-Identifier: MIT OR Apache-2.0
//! Recipient address format validation (DESIGN.md §6.1, §10.1).
//!
//! `tpt-statera` validates formats before generating a plan. `tpt-portus` checks them
//! again as defense in depth. Supported: EVM (EIP-55 checksum), Solana (base58, 32 bytes),
//! and Cosmos (bech32 with a 20- or 32-byte payload). Bitcoin is not yet supported.

use sha3::{Digest, Keccak256};
use thiserror::Error;

use crate::error::PortusError;
use crate::types::ChainId;

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum AddressError {
    #[error("unsupported chain for address validation")]
    UnsupportedChain,
    #[error("malformed address: {0}")]
    Malformed(&'static str),
    #[error("EIP-55 checksum mismatch")]
    BadChecksum,
}

/// Validate `address` for `chain` and return its decoded bytes.
pub fn validate_recipient(chain: &ChainId, address: &str) -> Result<Vec<u8>, PortusError> {
    let result = match chain {
        ChainId::Evm { .. } => validate_evm(address),
        ChainId::Solana { .. } => validate_solana(address),
        ChainId::Cosmos { .. } => validate_cosmos(address),
        ChainId::Bitcoin { .. } => Err(AddressError::UnsupportedChain),
    };
    result.map_err(|_| PortusError::InvalidAddressFormat {
        chain_id: chain.clone(),
    })
}

/// `0x`-prefixed 40-hex-digit address. Mixed-case input must carry a valid EIP-55 checksum.
pub fn validate_evm(address: &str) -> Result<Vec<u8>, AddressError> {
    let hex = address
        .strip_prefix("0x")
        .ok_or(AddressError::Malformed("missing 0x prefix"))?;
    if hex.len() != 40 {
        return Err(AddressError::Malformed("EVM address must be 40 hex digits"));
    }
    let bytes = hex_decode(hex).ok_or(AddressError::Malformed("non-hex character"))?;

    let has_upper = hex.bytes().any(|c| c.is_ascii_uppercase());
    let has_lower = hex.bytes().any(|c| c.is_ascii_lowercase());
    if has_upper && has_lower && hex != eip55_checksum(&hex.to_ascii_lowercase()) {
        return Err(AddressError::BadChecksum);
    }
    Ok(bytes)
}

/// Solana public key: base58 that decodes to exactly 32 bytes.
pub fn validate_solana(address: &str) -> Result<Vec<u8>, AddressError> {
    let bytes = base58_decode(address).ok_or(AddressError::Malformed("invalid base58"))?;
    if bytes.len() != 32 {
        return Err(AddressError::Malformed(
            "Solana address must decode to 32 bytes",
        ));
    }
    Ok(bytes)
}

/// Cosmos account: bech32 with a non-empty HRP and a 20- or 32-byte payload.
/// The HRP is not checked against a specific chain's prefix, because that mapping
/// is not in the spec yet.
pub fn validate_cosmos(address: &str) -> Result<Vec<u8>, AddressError> {
    let (hrp, payload) = bech32_decode(address)?;
    if hrp.is_empty() {
        return Err(AddressError::Malformed("empty bech32 HRP"));
    }
    if payload.len() != 20 && payload.len() != 32 {
        return Err(AddressError::Malformed(
            "Cosmos payload must be 20 or 32 bytes",
        ));
    }
    Ok(payload)
}

fn hex_decode(hex: &str) -> Option<Vec<u8>> {
    if !hex.bytes().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    (0..hex.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).ok())
        .collect()
}

/// EIP-55: uppercase the hex digit at position i when the i-th nibble of
/// keccak256(lowercase address) is >= 8.
fn eip55_checksum(lower_hex: &str) -> String {
    let hash = Keccak256::digest(lower_hex.as_bytes());
    lower_hex
        .char_indices()
        .map(|(i, c)| {
            let nibble = if i % 2 == 0 {
                hash[i / 2] >> 4
            } else {
                hash[i / 2] & 0x0f
            };
            if c.is_ascii_alphabetic() && nibble >= 8 {
                c.to_ascii_uppercase()
            } else {
                c
            }
        })
        .collect()
}

const BASE58_ALPHABET: &[u8] = b"123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz";

fn base58_decode(input: &str) -> Option<Vec<u8>> {
    if input.is_empty() {
        return None;
    }
    // Big-endian base-256 accumulator.
    let mut bytes: Vec<u8> = Vec::new();
    for c in input.bytes() {
        let mut carry = BASE58_ALPHABET.iter().position(|&a| a == c)? as u32;
        for b in bytes.iter_mut().rev() {
            carry += u32::from(*b) * 58;
            *b = carry as u8;
            carry >>= 8;
        }
        while carry > 0 {
            bytes.insert(0, carry as u8);
            carry >>= 8;
        }
    }
    // Each leading '1' encodes one leading zero byte.
    let leading_zeros = input.bytes().take_while(|&c| c == b'1').count();
    let significant: Vec<u8> = bytes.into_iter().skip_while(|&b| b == 0).collect();
    let mut out = vec![0u8; leading_zeros];
    out.extend(significant);
    Some(out)
}

const BECH32_CHARSET: &[u8] = b"qpzry9x8gf2tvdw0s3jn54khce6mua7l";

fn bech32_polymod(values: &[u8]) -> u32 {
    const GEN: [u32; 5] = [
        0x3b6a_57b2,
        0x2650_8e6d,
        0x1ea1_19fa,
        0x3d42_33dd,
        0x2a14_62b3,
    ];
    let mut chk: u32 = 1;
    for &v in values {
        let top = chk >> 25;
        chk = ((chk & 0x1ff_ffff) << 5) ^ u32::from(v);
        for (i, g) in GEN.iter().enumerate() {
            if (top >> i) & 1 == 1 {
                chk ^= g;
            }
        }
    }
    chk
}

fn bech32_hrp_expand(hrp: &str) -> Vec<u8> {
    hrp.bytes()
        .map(|c| c >> 5)
        .chain(std::iter::once(0))
        .chain(hrp.bytes().map(|c| c & 31))
        .collect()
}

/// Decode a BIP-173 bech32 string into (hrp, 8-bit payload).
fn bech32_decode(input: &str) -> Result<(String, Vec<u8>), AddressError> {
    if !(8..=90).contains(&input.len()) {
        return Err(AddressError::Malformed("bech32 length out of range"));
    }
    if input.bytes().any(|c| !(33..=126).contains(&c)) {
        return Err(AddressError::Malformed(
            "bech32 contains invalid characters",
        ));
    }
    let has_upper = input.bytes().any(|c| c.is_ascii_uppercase());
    let has_lower = input.bytes().any(|c| c.is_ascii_lowercase());
    if has_upper && has_lower {
        return Err(AddressError::Malformed("bech32 mixes upper and lower case"));
    }
    let input = input.to_ascii_lowercase();
    let sep = input
        .rfind('1')
        .ok_or(AddressError::Malformed("bech32 separator missing"))?;
    let (hrp, data_part) = (&input[..sep], &input[sep + 1..]);
    if data_part.len() < 6 {
        return Err(AddressError::Malformed("bech32 data part too short"));
    }
    let values: Vec<u8> = data_part
        .bytes()
        .map(|c| BECH32_CHARSET.iter().position(|&x| x == c).map(|p| p as u8))
        .collect::<Option<_>>()
        .ok_or(AddressError::Malformed("bech32 character outside charset"))?;

    let mut check = bech32_hrp_expand(hrp);
    check.extend_from_slice(&values);
    if bech32_polymod(&check) != 1 {
        return Err(AddressError::Malformed("bech32 checksum mismatch"));
    }
    let payload = convert_5_to_8(&values[..values.len() - 6])?;
    Ok((hrp.to_owned(), payload))
}

fn convert_5_to_8(groups: &[u8]) -> Result<Vec<u8>, AddressError> {
    let mut acc: u32 = 0;
    let mut bits: u32 = 0;
    let mut out = Vec::new();
    for &g in groups {
        acc = ((acc << 5) | u32::from(g)) & 0xfff;
        bits += 5;
        if bits >= 8 {
            bits -= 8;
            out.push(((acc >> bits) & 0xff) as u8);
        }
    }
    // Leftover bits must be fewer than 5 and all zero (no padding).
    if bits >= 5 || (acc & ((1 << bits) - 1)) != 0 {
        return Err(AddressError::Malformed("bech32 has invalid padding"));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::BitcoinNetwork;

    #[test]
    fn evm_accepts_lowercase_and_valid_checksum() {
        assert!(validate_evm("0x5aaeb6053f3e94c9b9a09f33669435e7ef1beaed").is_ok());
        assert!(validate_evm("0x5aAeb6053F3E94C9b9A09f33669435E7Ef1BeAed").is_ok());
    }

    #[test]
    fn evm_rejects_bad_checksum_and_format() {
        assert_eq!(
            validate_evm("0x5aAeb6053f3E94C9b9A09f33669435E7Ef1BeAed"),
            Err(AddressError::BadChecksum)
        );
        assert!(validate_evm("5aAeb6053F3E94C9b9A09f33669435E7Ef1BeAed").is_err());
        assert!(validate_evm("0x1234").is_err());
        assert!(validate_evm("0xzz").is_err());
    }

    #[test]
    fn solana_system_program_is_32_zero_bytes() {
        let bytes = validate_solana("11111111111111111111111111111111").unwrap();
        assert_eq!(bytes, vec![0u8; 32]);
    }

    #[test]
    fn solana_rejects_bad_alphabet_and_length() {
        assert!(validate_solana("0OIl0OIl0OIl0OIl0OIl0OIl0OIl0OIl").is_err());
        assert!(validate_solana("1111").is_err());
    }

    #[test]
    fn bech32_bip173_vectors() {
        assert!(bech32_decode("A12UEL5L").is_ok());
        assert!(bech32_decode("abcdef1qpzry9x8gf2tvdw0s3jn54khce6mua7lmqqqxw").is_ok());
        assert!(bech32_decode("abcdef1qpzry9x8gf2tvdw0s3jn54khce6mua7lmqqqxv").is_err());
        assert!(bech32_decode("abcdef1qpzry9x8gf2tvdw0s3jn54khce6mua7lMqqqxw").is_err());
    }

    fn bech32_encode(hrp: &str, payload: &[u8]) -> String {
        let mut data5 = Vec::new();
        let (mut acc, mut bits) = (0u32, 0u32);
        for &b in payload {
            acc = (acc << 8) | u32::from(b);
            bits += 8;
            while bits >= 5 {
                bits -= 5;
                data5.push(((acc >> bits) & 31) as u8);
            }
        }
        if bits > 0 {
            data5.push(((acc << (5 - bits)) & 31) as u8);
        }
        let mut check = bech32_hrp_expand(hrp);
        check.extend_from_slice(&data5);
        check.extend_from_slice(&[0; 6]);
        let m = bech32_polymod(&check) ^ 1;
        let mut out = format!("{hrp}1");
        for &d in &data5 {
            out.push(BECH32_CHARSET[d as usize] as char);
        }
        for i in 0..6 {
            out.push(BECH32_CHARSET[((m >> (5 * (5 - i))) & 31) as usize] as char);
        }
        out
    }

    #[test]
    fn cosmos_round_trip_20_and_32_byte_payloads() {
        let a20 = bech32_encode("cosmos", &[7u8; 20]);
        assert_eq!(validate_cosmos(&a20).unwrap(), vec![7u8; 20]);
        let a32 = bech32_encode("osmo", &[9u8; 32]);
        assert_eq!(validate_cosmos(&a32).unwrap(), vec![9u8; 32]);
        assert!(validate_cosmos(&bech32_encode("cosmos", &[1u8; 16])).is_err());
    }

    #[test]
    fn base58_leading_ones_become_zero_bytes() {
        assert_eq!(base58_decode("1").unwrap(), vec![0u8]);
        assert_eq!(base58_decode("2").unwrap(), vec![1u8]);
    }

    #[test]
    fn recipient_validation_maps_errors_to_portus_error() {
        let chain = ChainId::Evm { eip155_id: 1 };
        assert_eq!(
            validate_recipient(&chain, "nope"),
            Err(PortusError::InvalidAddressFormat { chain_id: chain })
        );
        assert!(validate_recipient(
            &ChainId::Bitcoin {
                network: BitcoinNetwork::Mainnet
            },
            "x"
        )
        .is_err());
    }
}
