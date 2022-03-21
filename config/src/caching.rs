//! Support types for configuring storage caching

use crate::Chain;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::{fmt, str::FromStr};

/// Settings to configure caching of remote
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct StorageCachingConfig {
    /// chains to cache
    pub chains: CachedChains,
    /// endpoints to cache
    pub endpoints: CachedEndpoints,
}

/// What chains to cache
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CachedChains {
    /// Cache all chains
    All,
    /// Only cache these chains
    Chains(Vec<Chain>),
}

impl Serialize for CachedChains {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match self {
            CachedChains::All => serializer.serialize_str("all"),
            CachedChains::Chains(chains) => chains.serialize(serializer),
        }
    }
}

impl<'de> Deserialize<'de> for CachedChains {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Chains {
            All(String),
            Chains(Vec<Chain>),
        }

        match Chains::deserialize(deserializer)? {
            Chains::All(_) => Ok(CachedChains::All),
            Chains::Chains(chains) => Ok(CachedChains::Chains(chains)),
        }
    }
}

impl Default for CachedChains {
    fn default() -> Self {
        CachedChains::All
    }
}

/// What endpoints to enable caching for
#[derive(Debug, Clone)]
pub enum CachedEndpoints {
    /// Cache all endpoints
    All,
    /// Only cache non-local host endpoints
    Remote,
    /// Only cache these chains
    Pattern(regex::Regex),
}

impl PartialEq for CachedEndpoints {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (&CachedEndpoints::Pattern(ref a), &CachedEndpoints::Pattern(ref b)) => {
                a.as_str() == b.as_str()
            }
            (&CachedEndpoints::All, &CachedEndpoints::All) => true,
            (&CachedEndpoints::Remote, &CachedEndpoints::Remote) => true,
            _ => false,
        }
    }
}

impl Eq for CachedEndpoints {}

impl Default for CachedEndpoints {
    fn default() -> Self {
        CachedEndpoints::Remote
    }
}

impl fmt::Display for CachedEndpoints {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CachedEndpoints::All => f.write_str("all"),
            CachedEndpoints::Remote => f.write_str("remote"),
            CachedEndpoints::Pattern(s) => s.fmt(f),
        }
    }
}

impl FromStr for CachedEndpoints {
    type Err = regex::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "all" => Ok(CachedEndpoints::All),
            "remote" => Ok(CachedEndpoints::Remote),
            _ => Ok(CachedEndpoints::Pattern(s.parse()?)),
        }
    }
}

impl<'de> Deserialize<'de> for CachedEndpoints {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        String::deserialize(deserializer)?.parse().map_err(serde::de::Error::custom)
    }
}

impl Serialize for CachedEndpoints {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match self {
            CachedEndpoints::All => serializer.serialize_str("all"),
            CachedEndpoints::Remote => serializer.serialize_str("remote"),
            CachedEndpoints::Pattern(pattern) => serializer.serialize_str(pattern.as_str()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn can_parse_storage_config() {
        #[derive(Serialize, Deserialize)]
        pub struct Wrapper {
            pub rpc_storage_caching: StorageCachingConfig,
        }

        let s = r#"rpc_storage_caching = { chains = "all", endpoints = "remote"}"#;
        let w: Wrapper = toml::from_str(s).unwrap();

        assert_eq!(
            w.rpc_storage_caching,
            StorageCachingConfig { chains: CachedChains::All, endpoints: CachedEndpoints::Remote }
        );

        let s = r#"rpc_storage_caching = { chains = [1, "optimism", 999999], endpoints = "all"}"#;
        let w: Wrapper = toml::from_str(s).unwrap();

        assert_eq!(
            w.rpc_storage_caching,
            StorageCachingConfig {
                chains: CachedChains::Chains(vec![
                    Chain::Named(ethers_core::types::Chain::Mainnet),
                    Chain::Named(ethers_core::types::Chain::Optimism),
                    Chain::Id(999999)
                ]),
                endpoints: CachedEndpoints::All
            }
        )
    }
}
