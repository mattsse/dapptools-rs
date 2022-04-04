//! Verify contract source on etherscan

use crate::{
    cmd::forge::{build::BuildArgs, flatten::CoreFlattenArgs},
    opts::forge::ContractInfo,
};
use clap::Parser;
use ethers::{
    abi::Address,
    etherscan::{contract::VerifyContract, Client},
    solc::{artifacts::Source, AggregatedCompilerOutput, CompilerInput, Solc},
};
use semver::Version;
use std::collections::BTreeMap;

/// Verification arguments
#[derive(Debug, Clone, Parser)]
pub struct VerifyArgs {
    #[clap(help = "the target contract address")]
    address: Address,

    #[clap(help = "the contract source info `<path>:<contractname>`")]
    contract: ContractInfo,

    #[clap(long, help = "the encoded constructor arguments")]
    constructor_args: Option<String>,

    #[clap(long, help = "the compiler version used during build")]
    compiler_version: String,

    #[clap(long, help = "the number of optimization runs used")]
    num_of_optimizations: Option<u32>,

    // TODO: Allow choosing network using the provider or chainid as string
    #[clap(long, help = "the chain id of the network you are verifying for", default_value = "1")]
    chain_id: u64,

    #[clap(help = "your etherscan api key", env = "ETHERSCAN_API_KEY")]
    etherscan_key: String,

    #[clap(flatten)]
    opts: CoreFlattenArgs,

    #[clap(
        short,
        long,
        help = r#"usually the command will try to compile the flattened code locally first to ensure it's valid.
This flag we skip that process and send the content directly to the endpoint."#
    )]
    force: bool,
}

impl VerifyArgs {
    /// Run the verify command to submit the contract's source code for verification on etherscan
    pub async fn run(&self) -> eyre::Result<()> {
        if self.contract.path.is_none() {
            eyre::bail!("Contract info must be provided in the format <path>:<name>")
        }

        let CoreFlattenArgs {
            root,
            contracts,
            remappings,
            remappings_env,
            cache_path,
            lib_paths,
            hardhat,
        } = self.opts.clone();

        let build_args = BuildArgs {
            root,
            contracts,
            remappings,
            remappings_env,
            cache_path,
            lib_paths,
            out_path: None,
            compiler: Default::default(),
            names: false,
            sizes: false,
            ignored_error_codes: vec![],
            no_auto_detect: false,
            use_solc: None,
            offline: false,
            force: false,
            hardhat,
            libraries: vec![],
            watch: Default::default(),
            via_ir: false,
            config_path: None,
        };

        let project = build_args.project()?;
        let contract = project
            .flatten(&project.root().join(self.contract.path.as_ref().unwrap()))
            .map_err(|err| eyre::eyre!("Failed to flatten contract: {}", err))?;

        if !self.force {
            // solc dry run
            self.check_flattened(contract.clone()).await.map_err(|err| {
                eyre::eyre!(
                    "Failed to compile the flattened code locally: `{}`\
To skip this solc dry, have a look at the  `--force` flag of this command.",
                    err
                )
            })?;
        }

        let etherscan = Client::new(self.chain_id.try_into()?, &self.etherscan_key)
            .map_err(|err| eyre::eyre!("Failed to create etherscan client: {}", err))?;

        let mut verify_args = VerifyContract::new(
            self.address,
            self.contract.name.clone(),
            contract,
            self.compiler_version.clone(),
        )
        .constructor_arguments(self.constructor_args.clone());

        if let Some(optimizations) = self.num_of_optimizations {
            verify_args = verify_args.optimization(true).runs(optimizations);
        } else {
            verify_args = verify_args.optimization(false);
        }

        let resp = etherscan
            .submit_contract_verification(&verify_args)
            .await
            .map_err(|err| eyre::eyre!("Failed to submit contract verification: {}", err))?;

        if resp.status == "0" {
            if resp.message == "Contract source code already verified" {
                println!("Contract source code already verified.");
                return Ok(())
            }

            eyre::bail!(
                "Encountered an error verifying this contract:\nResponse: `{}`\nDetails: `{}`",
                resp.message,
                resp.result
            );
        }

        println!(
            r#"Submitted contract for verification:
                Response: `{}`
                GUID: `{}`
                url: {}#code"#,
            resp.message,
            resp.result,
            etherscan.address_url(self.address)
        );
        Ok(())
    }

    /// Attempts to compile the flattened content locally with the compiler version
    async fn check_flattened(&self, content: impl Into<String>) -> eyre::Result<()> {
        let version: Version = self.compiler_version.parse()?;
        let solc = if let Some(solc) = Solc::find_svm_installed_version(&self.compiler_version)? {
            solc
        } else {
            Solc::install(&version).await?
        };
        let input = CompilerInput {
            language: "Solidity".to_string(),
            sources: BTreeMap::from([("constract.sol".into(), Source { content: content.into() })]),
            settings: Default::default(),
        };

        let out = solc.compile(&input)?;
        if out.has_error() {
            let mut o = AggregatedCompilerOutput::default();
            o.extend(version, out);
            eprintln!("{}", o.diagnostics(&[]));

            eprintln!(
                r#"Failed to compile the flattened code locally.
This could be a bug, please inspect the outout of `forge flatten {}` and report an issue.
To skip this solc dry, have a look at the  `--force` flag of this command.
"#,
                self.contract.path.as_ref().expect("Path is some;")
            );
            std::process::exit(1)
        }

        Ok(())
    }
}

/// Check verification status arguments
#[derive(Debug, Clone, Parser)]
pub struct VerifyCheckArgs {
    #[clap(help = "the verification guid")]
    guid: String,

    // TODO: Allow choosing network using the provider or chainid as string
    #[clap(long, help = "the chain id of the network you are verifying for", default_value = "1")]
    chain_id: u64,

    #[clap(help = "your etherscan api key", env = "ETHERSCAN_API_KEY")]
    etherscan_key: String,
}

impl VerifyCheckArgs {
    /// Executes the command to check verification status on Etherscan
    pub async fn run(&self) -> eyre::Result<()> {
        let etherscan = Client::new(self.chain_id.try_into()?, &self.etherscan_key)
            .map_err(|err| eyre::eyre!("Failed to create etherscan client: {}", err))?;

        let resp = etherscan
            .check_contract_verification_status(self.guid.clone())
            .await
            .map_err(|err| eyre::eyre!("Failed to request verification status: {}", err))?;

        if resp.status == "0" {
            if resp.result == "Pending in queue" {
                println!("Verification is pending...");
                return Ok(())
            }

            eyre::bail!(
                "Contract verification failed:\nResponse: `{}`\nDetails: `{}`",
                resp.message,
                resp.result
            );
        }

        println!("Contract successfully verified.");
        Ok(())
    }
}
