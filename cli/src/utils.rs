use std::{
    future::Future,
    path::{Path, PathBuf},
    str::FromStr,
    time::Duration,
};

use ethers::{
    providers::{Middleware, Provider},
    solc::EvmVersion,
    types::U256,
};
use forge::executor::{opts::EvmOpts, Fork, SpecId};
use foundry_config::{caching::StorageCachingConfig, Config};
// reexport all `foundry_config::utils`
#[doc(hidden)]
pub use foundry_config::utils::*;

/// The version message for the current program, like
/// `forge 0.1.0 (f01b232bc 2022-01-22T23:28:39.493201+00:00)`
pub(crate) const VERSION_MESSAGE: &str = concat!(
    env!("CARGO_PKG_VERSION"),
    " (",
    env!("VERGEN_GIT_SHA_SHORT"),
    " ",
    env!("VERGEN_BUILD_TIMESTAMP"),
    ")"
);

/// Useful extensions to [`std::path::Path`].
pub trait FoundryPathExt {
    /// Returns true if the [`Path`] ends with `.t.sol`
    fn is_sol_test(&self) -> bool;

    /// Returns true if the  [`Path`] has a `sol` extension
    fn is_sol(&self) -> bool;

    /// Returns true if the  [`Path`] has a `yul` extension
    fn is_yul(&self) -> bool;
}

impl<T: AsRef<Path>> FoundryPathExt for T {
    fn is_sol_test(&self) -> bool {
        self.as_ref()
            .file_name()
            .and_then(|s| s.to_str())
            .map(|s| s.ends_with(".t.sol"))
            .unwrap_or_default()
    }

    fn is_sol(&self) -> bool {
        self.as_ref().extension() == Some(std::ffi::OsStr::new("sol"))
    }

    fn is_yul(&self) -> bool {
        self.as_ref().extension() == Some(std::ffi::OsStr::new("yul"))
    }
}

/// Initializes a tracing Subscriber for logging
#[allow(dead_code)]
pub fn subscriber() {
    tracing_subscriber::FmtSubscriber::builder()
        // .with_timer(tracing_subscriber::fmt::time::uptime())
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();
}

pub fn evm_spec(evm: &EvmVersion) -> SpecId {
    match evm {
        EvmVersion::Istanbul => SpecId::ISTANBUL,
        EvmVersion::Berlin => SpecId::BERLIN,
        EvmVersion::London => SpecId::LONDON,
        _ => panic!("Unsupported EVM version"),
    }
}

/// Securely reads a secret from stdin, or proceeds to return a fallback value
/// which was provided in cleartext via CLI or env var
#[allow(dead_code)]
pub fn read_secret(secret: bool, unsafe_secret: Option<String>) -> eyre::Result<String> {
    Ok(if secret {
        println!("Insert secret:");
        rpassword::read_password()?
    } else {
        // guaranteed to be Some(..)
        unsafe_secret.unwrap()
    })
}

/// Artifact/Contract identifier can take the following form:
/// `<artifact file name>:<contract name>`, the `artifact file name` is the name of the json file of
/// the contract's artifact and the contract name is the name of the solidity contract, like
/// `SafeTransferLibTest.json:SafeTransferLibTest`
///
/// This returns the `contract name` part
///
/// # Example
///
/// ```
/// assert_eq!(
///     "SafeTransferLibTest",
///     utils::get_contract_name("SafeTransferLibTest.json:SafeTransferLibTest")
/// );
/// ```
pub fn get_contract_name(id: &str) -> &str {
    id.rsplit(':').next().unwrap_or(id)
}

/// This returns the `file name` part, See [`get_contract_name`]
///
/// # Example
///
/// ```
/// assert_eq!(
///     "SafeTransferLibTest.json",
///     utils::get_file_name("SafeTransferLibTest.json:SafeTransferLibTest")
/// );
/// ```
pub fn get_file_name(id: &str) -> &str {
    id.split(':').next().unwrap_or(id)
}

/// parse a hex str or decimal str as U256
pub fn parse_u256(s: &str) -> eyre::Result<U256> {
    Ok(if s.starts_with("0x") { U256::from_str(s)? } else { U256::from_dec_str(s)? })
}

/// Parses a `Duration` from a &str
pub fn parse_delay(delay: &str) -> eyre::Result<Duration> {
    let delay = if delay.ends_with("ms") {
        let d: u64 = delay.trim_end_matches("ms").parse()?;
        Duration::from_millis(d)
    } else {
        let d: f64 = delay.parse()?;
        let delay = (d * 1000.0).round();
        if delay.is_infinite() || delay.is_nan() || delay.is_sign_negative() {
            eyre::bail!("delay must be finite and non-negative");
        }

        Duration::from_millis(delay as u64)
    };
    Ok(delay)
}

/// Runs the `future` in a new [`tokio::runtime::Runtime`]
#[allow(unused)]
pub fn block_on<F: Future>(future: F) -> F::Output {
    let rt = tokio::runtime::Runtime::new().expect("could not start tokio rt");
    rt.block_on(future)
}

/// Helper function that returns the [Fork] to use, if any.
pub fn get_fork(evm_opts: &EvmOpts, config: &StorageCachingConfig) -> Option<Fork> {
    fn get_cache_storage_path(
        evm_opts: &EvmOpts,
        config: &StorageCachingConfig,
    ) -> Option<PathBuf> {
        if evm_opts.no_storage_cache {
            // storage caching explicitly opted out of
            return None
        }
        let url = evm_opts.fork_url.as_ref()?;
        // cache only if block explicitly pinned
        let block = evm_opts.fork_block_number?;
        if config.enable_for_endpoint(url) {
            // also need to get the chain id to compute the cache path
            let provider = Provider::try_from(url.as_str()).expect("Failed to establish provider");
            match block_on(provider.get_chainid()) {
                Ok(chain_id) => {
                    let chain_id: u64 = chain_id.try_into().ok()?;
                    if config.enable_for_chain_id(chain_id) {
                        let chain = if let Ok(chain) = ethers::types::Chain::try_from(chain_id) {
                            chain.to_string()
                        } else {
                            format!("{}", chain_id)
                        };
                        return Some(Config::data_dir().ok()?.join(chain).join(format!("{}", block)))
                    }
                }
                Err(err) => {
                    tracing::warn!("Failed to get chain id for {}: {:?}", url, err);
                }
            }
        }

        None
    }

    if let Some(ref url) = evm_opts.fork_url {
        let cache_storage = get_cache_storage_path(evm_opts, config);
        let fork = Fork { url: url.clone(), pin_block: evm_opts.fork_block_number, cache_storage };
        return Some(fork)
    }

    None
}

/// Conditionally print a message
///
/// This macro accepts a predicate and the message to print if the predicate is tru
///
/// ```rust
/// let quiet = true;
/// p_println!(!quiet => "message");
/// ```
macro_rules! p_println {
    ($p:expr => $($arg:tt)*) => {{
        if $p {
            println!($($arg)*)
        }
    }}
}
pub(crate) use p_println;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn foundry_path_ext_works() {
        let p = Path::new("contracts/MyTest.t.sol");
        assert!(p.is_sol_test());
        assert!(p.is_sol());
        let p = Path::new("contracts/Greeter.sol");
        assert!(!p.is_sol_test());
    }
}
