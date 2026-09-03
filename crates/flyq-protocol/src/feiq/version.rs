//! FeiQ extended version string parsing.
//!
//! Standard IPMsg uses `"1"` as the version field.
//! FeiQ replaces it with a structured string:
//!
//! ```text
//! 1_lbt<N>_<M>#level#mac#0#0#0#4001#9
//! ```
//!
//! Where:
//! - `N` = protocol generation (4 = FeiQ 2.x, 6 = FeiQ 3.x)
//! - `M` = sub-version
//! - `level` = user rank/level (cosmetic, drives QQ-style rank icons)
//! - `mac` = MAC address (12-char hex, used for user identity and group chat encryption)
//! - Remaining fields are undocumented constants

use serde::{Deserialize, Serialize};

/// FeiQ protocol generation (from the `lbt<N>` field).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProtocolGeneration {
    /// Standard IPMsg (no FeiQ extension).
    StandardIpMsg,
    /// FeiQ 2.x (lbt4).
    Feiq2,
    /// FeiQ 3.x (lbt6).
    Feiq3,
    /// Unknown FeiQ generation.
    Unknown(u32),
}

/// Parsed FeiQ version string.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FeiqVersion {
    /// Protocol generation.
    pub generation: ProtocolGeneration,
    /// Sub-version number.
    pub sub_version: u32,
    /// User rank/level (cosmetic).
    pub level: u32,
    /// MAC address as hex string (12 chars, e.g., "74E6E2152F9D").
    pub mac: String,
    /// Raw version string (preserved for re-transmission).
    pub raw: String,
}

impl FeiqVersion {
    /// Parse a version string from an IPMsg packet.
    ///
    /// Handles both standard IPMsg (`"1"`) and FeiQ extended versions.
    pub fn parse(version_str: &str) -> Self {
        // Check if it's a FeiQ extended version (contains "_lbt")
        if !version_str.contains("_lbt") {
            return Self {
                generation: ProtocolGeneration::StandardIpMsg,
                sub_version: version_str.parse().unwrap_or(1),
                level: 0,
                mac: String::new(),
                raw: version_str.to_string(),
            };
        }

        // Parse FeiQ version: "1_lbt6_0#128#74E6E2152F9D#0#0#0#4001#9"
        let fields: Vec<&str> = version_str.split('#').collect();

        // First field: "1_lbt<N>_<M>"
        let header = fields[0];
        let (generation, sub_version) = Self::parse_header(header);

        // Second field: level
        let level = fields.get(1).and_then(|s| s.parse::<u32>().ok()).unwrap_or(0);

        // Third field: MAC address
        let mac = fields.get(2).map(|s| s.to_uppercase()).unwrap_or_default();

        Self {
            generation,
            sub_version,
            level,
            mac,
            raw: version_str.to_string(),
        }
    }

    /// Parse the header portion "1_lbt<N>_<M>".
    fn parse_header(header: &str) -> (ProtocolGeneration, u32) {
        // Expected format: "1_lbt6_0" or "1_lbt4_10"
        // split('_') gives ["1", "lbt6", "0"] or ["1", "lbt4", "10"]
        let parts: Vec<&str> = header.split('_').collect();
        if parts.len() >= 2 && parts[1].starts_with("lbt") {
            let gen_str = &parts[1][3..]; // strip "lbt" prefix
            let gen_num = gen_str.parse::<u32>().unwrap_or(0);
            let sub_ver = parts.get(2).and_then(|s| s.parse::<u32>().ok()).unwrap_or(0);
            let generation = match gen_num {
                4 => ProtocolGeneration::Feiq2,
                6 => ProtocolGeneration::Feiq3,
                n => ProtocolGeneration::Unknown(n),
            };
            (generation, sub_ver)
        } else {
            (ProtocolGeneration::StandardIpMsg, 1)
        }
    }

    /// Check if this is a FeiQ client (not standard IPMsg).
    pub fn is_feiq(&self) -> bool {
        !matches!(self.generation, ProtocolGeneration::StandardIpMsg)
    }

    /// Build a FeiQ 3.x version string for broadcasting.
    pub fn build_feiq3(mac: &str, level: u32) -> String {
        format!("1_lbt6_0#{}#{}#0#0#0#4001#9", level, mac.to_uppercase())
    }

    /// Build a standard IPMsg version string.
    pub fn build_standard() -> String {
        "1".to_string()
    }

    /// Get the MAC address in a format suitable for Blowfish key (group chat).
    /// Returns the 12-char uppercase hex MAC, or empty string if not available.
    pub fn blowfish_key(&self) -> &str {
        &self.mac
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_standard_version() {
        let v = FeiqVersion::parse("1");
        assert_eq!(v.generation, ProtocolGeneration::StandardIpMsg);
        assert!(!v.is_feiq());
        assert_eq!(v.mac, "");
    }

    #[test]
    fn test_parse_feiq3_version() {
        let v = FeiqVersion::parse("1_lbt6_0#128#74E6E2152F9D#0#0#0#4001#9");
        assert_eq!(v.generation, ProtocolGeneration::Feiq3);
        assert!(v.is_feiq());
        assert_eq!(v.level, 128);
        assert_eq!(v.mac, "74E6E2152F9D");
        assert_eq!(v.sub_version, 0);
    }

    #[test]
    fn test_parse_feiq2_version() {
        let v = FeiqVersion::parse("1_lbt4_10#65664#002481627512#0#0#0");
        assert_eq!(v.generation, ProtocolGeneration::Feiq2);
        assert!(v.is_feiq());
        assert_eq!(v.level, 65664);
        assert_eq!(v.mac, "002481627512");
        assert_eq!(v.sub_version, 10);
    }

    #[test]
    fn test_parse_feix_version() {
        let v = FeiqVersion::parse("1_lbt6_8#998#FeiX#0#0#0#4001#9");
        assert_eq!(v.generation, ProtocolGeneration::Feiq3);
        assert_eq!(v.level, 998);
        assert_eq!(v.mac, "FEIX"); // uppercase
    }

    #[test]
    fn test_build_feiq3() {
        let s = FeiqVersion::build_feiq3("74e6e2152f9d", 128);
        assert_eq!(s, "1_lbt6_0#128#74E6E2152F9D#0#0#0#4001#9");
    }

    #[test]
    fn test_blowfish_key() {
        let v = FeiqVersion::parse("1_lbt6_0#128#74E6E2152F9D#0#0#0#4001#9");
        assert_eq!(v.blowfish_key(), "74E6E2152F9D");
    }
}
