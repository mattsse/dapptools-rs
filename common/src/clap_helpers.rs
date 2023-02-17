//! Additional utils for clap

use ethers_core::types::NameOrAddress;
use std::str::FromStr;

/// A `clap` `value_parser` that removes a `0x` prefix if it exists
pub fn strip_0x_prefix(s: &str) -> Result<String, &'static str> {
    Ok(s.strip_prefix("0x").unwrap_or(s).to_string())
}

/// A `clap` `value_parser` that ensures [NameOrAddress] is parsed via [FromStr].
///
/// By default `From<&str>` takes precedence over `FromStr` in clap.
pub fn parse_name_or_address(s: &str) -> Result<NameOrAddress, <NameOrAddress as FromStr>::Err> {
    s.parse()
}
