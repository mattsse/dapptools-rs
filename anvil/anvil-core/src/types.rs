use ethers_core::types::{BlockNumber, H256, U256};
use serde::{
    de::{Error, Visitor},
    Deserialize, Deserializer, Serialize, Serializer,
};
use std::fmt;

/// Represents the params to set forking
#[derive(Clone, Debug, PartialEq, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Forking {
    json_rpc_url: Option<String>,
    block_number: Option<BlockNumber>,
}

/// Additional `evm_mine` options
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(untagged)]
pub enum EvmMineOptions {
    Options {
        timestamp: Option<u64>,
        // If `blocks` is given, it will mine exactly blocks number of blocks, regardless of any
        // other blocks mined or reverted during it's operation
        blocks: Option<u64>,
    },
    /// The timestamp the block should be mined with
    Timestamp(Option<u64>),
}

/// Represents the result of `eth_getWork`
/// This may or may not include the block number
#[derive(Debug, PartialEq, Eq, Default)]
pub struct Work {
    pub pow_hash: H256,
    pub seed_hash: H256,
    pub target: H256,
    pub number: Option<u64>,
}

impl Serialize for Work {
    fn serialize<S>(&self, s: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        if let Some(num) = self.number {
            (&self.pow_hash, &self.seed_hash, &self.target, U256::from(num)).serialize(s)
        } else {
            (&self.pow_hash, &self.seed_hash, &self.target).serialize(s)
        }
    }
}

/// A hex encoded or decimal index
#[derive(Debug, PartialEq, Eq, Hash, Clone, Copy)]
pub struct Index(usize);

impl From<Index> for usize {
    fn from(idx: Index) -> Self {
        idx.0
    }
}

impl<'a> Deserialize<'a> for Index {
    fn deserialize<D>(deserializer: D) -> Result<Index, D::Error>
    where
        D: Deserializer<'a>,
    {
        struct IndexVisitor;

        impl<'a> Visitor<'a> for IndexVisitor {
            type Value = Index;

            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                write!(formatter, "hex-encoded or decimal index")
            }

            fn visit_u64<E>(self, value: u64) -> Result<Self::Value, E>
            where
                E: Error,
            {
                Ok(Index(value as usize))
            }

            fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
            where
                E: Error,
            {
                if let Some(val) = value.strip_prefix("0x") {
                    usize::from_str_radix(val, 16).map(Index).map_err(|e| {
                        Error::custom(format!("Failed to parse hex encoded index value: {}", e))
                    })
                } else {
                    value
                        .parse::<usize>()
                        .map(Index)
                        .map_err(|e| Error::custom(format!("Failed to parse numeric index: {}", e)))
                }
            }

            fn visit_string<E>(self, value: String) -> Result<Self::Value, E>
            where
                E: Error,
            {
                self.visit_str(value.as_ref())
            }
        }

        deserializer.deserialize_any(IndexVisitor)
    }
}
