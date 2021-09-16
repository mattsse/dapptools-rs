use ethers::core::utils::{CompiledContract, Solc};
use eyre::Result;
use semver::{Version, VersionReq};
use std::{
    collections::HashMap,
    fs::File,
    io::{BufRead, BufReader},
    path::{Path, PathBuf},
    time::Instant,
};

/// Supports building contracts
#[derive(Clone, Debug)]
pub struct SolcBuilder<'a> {
    contracts: &'a str,
    remappings: &'a [String],
    lib_paths: &'a [String],
    versions: Vec<Version>,
    releases: Vec<Version>,
}

impl<'a> SolcBuilder<'a> {
    pub fn new(
        contracts: &'a str,
        remappings: &'a [String],
        lib_paths: &'a [String],
    ) -> Result<Self> {
        let versions = svm::installed_versions().unwrap_or_default();
        // Try to download the releases, if it fails default to empty
        let releases = match tokio::runtime::Runtime::new()?.block_on(svm::all_versions()) {
            Ok(inner) => inner,
            Err(err) => {
                tracing::error!("Failed to get upstream releases: {}", err);
                Vec::new()
            }
        };
        Ok(Self {
            contracts,
            remappings,
            lib_paths,
            versions,
            releases,
        })
    }

    /// Builds all provided contract files with the specified compiler version.
    /// Assumes that the lib-paths and remappings have already been specified and
    /// that the correct compiler version is provided.
    #[tracing::instrument(skip(self, files))]
    fn build(
        &self,
        version: &str,
        files: Vec<String>,
    ) -> Result<HashMap<String, CompiledContract>> {
        let compiler_path = find_installed_version_path(version)?
            .ok_or_else(|| eyre::eyre!("version {} not installed", version))?;

        // tracing::trace!(?files);
        let mut solc = Solc::new_with_paths(files).solc_path(compiler_path);
        let lib_paths = self
            .lib_paths
            .iter()
            .filter(|path| PathBuf::from(path).exists())
            .map(|path| {
                std::fs::canonicalize(path)
                    .unwrap()
                    .into_os_string()
                    .into_string()
                    .unwrap()
            })
            .collect::<Vec<_>>()
            .join(",");

        // tracing::trace!(?lib_paths);
        solc = solc.args(["--allow-paths", &lib_paths]);

        // tracing::trace!(?self.remappings);
        if !self.remappings.is_empty() {
            solc = solc.args(self.remappings)
        }

        Ok(solc.build()?)
    }

    /// Builds all contracts with their corresponding compiler versions
    #[tracing::instrument(skip(self))]
    pub fn build_all(&mut self) -> Result<HashMap<String, CompiledContract>> {
        let contracts_by_version = self.contract_versions()?;

        let start = Instant::now();
        let res = contracts_by_version.into_iter().try_fold(
            HashMap::new(),
            |mut map, (version, files)| {
                let res = self.build(&version, files)?;
                map.extend(res);
                Ok::<_, eyre::Error>(map)
            },
        );
        let duration = Instant::now().duration_since(start);
        tracing::info!(compilation_time = ?duration);

        res
    }

    /// Given a Solidity file, it detects the latest compiler version which can be used
    /// to build it, and returns it along with its canonicalized path. If the required
    /// compiler version is not installed, it also proceeds to install it.
    fn detect_version(&mut self, fname: &Path) -> Result<Option<(Version, String)>> {
        let path = std::fs::canonicalize(fname)?;

        // detects the required solc version
        let sol_version = Self::version_req(&path)?;

        let path_str = path
            .into_os_string()
            .into_string()
            .map_err(|_| eyre::eyre!("invalid path, maybe not utf-8?"))?;

        // use the installed one, install it if it does not exist
        let res = Self::find_matching_installation(&mut self.versions, &sol_version)
            .or_else(|| {
                // Check upstream for a matching install
                Self::find_matching_installation(&mut self.releases, &sol_version).map(|version| {
                    println!("Installing {}", version);
                    // Blocking call to install it over RPC.
                    install_blocking(&version).expect("could not install solc remotely");
                    self.versions.push(version.clone());
                    println!("Done!");
                    version
                })
            })
            .map(|version| (version, path_str));

        Ok(res)
    }

    /// Gets a map of compiler version -> vec[contract paths]
    fn contract_versions(&mut self) -> Result<HashMap<String, Vec<String>>> {
        // Group contracts in the nones with the same version pragma
        let files = glob::glob(self.contracts)?;
        // tracing::trace!("Compiling files under {}", self.contracts);
        println!("Compiling files under {}", self.contracts);

        // get all the corresponding contract versions
        Ok(files
            .filter_map(|fname| fname.ok())
            .filter_map(|fname| self.detect_version(&fname).ok().flatten())
            .fold(HashMap::new(), |mut map, (version, path)| {
                let entry = map.entry(version.to_string()).or_insert_with(Vec::new);
                entry.push(path);
                map
            }))
    }

    /// Parses the given Solidity file looking for the `pragma` definition and
    /// returns the corresponding SemVer version requirement.
    fn version_req(path: &Path) -> Result<VersionReq> {
        let file = BufReader::new(File::open(path)?);
        let version = file
            .lines()
            .map(|line| line.unwrap())
            .find(|line| line.starts_with("pragma"))
            .ok_or_else(|| eyre::eyre!("{:?} has no version", path))?;
        let version = version
            .replace("pragma solidity ", "")
            .replace(";", "")
            // needed to make it valid semver for things like
            // >=0.4.0 <0.5.0
            .replace(" ", ",");

        Ok(VersionReq::parse(&version)?)
    }

    /// Find a matching local installation for the specified required version
    fn find_matching_installation(
        versions: &mut [Version],
        required_version: &VersionReq,
    ) -> Option<Version> {
        // sort through them
        versions.sort();
        // iterate in reverse to find the last match
        versions
            .iter()
            .rev()
            .find(|version| required_version.matches(version))
            .cloned()
    }
}

/// Returns the path for an installed version
fn find_installed_version_path(version: &str) -> Result<Option<PathBuf>> {
    let home_dir = svm::SVM_HOME.clone();
    let path = std::fs::read_dir(home_dir)?
        .into_iter()
        .filter_map(|version| version.ok())
        .map(|version_dir| version_dir.path())
        .find(|path| path.to_string_lossy().contains(&version))
        .map(|mut path| {
            path.push(format!("solc-{}", &version));
            path
        });
    Ok(path)
}

/// Blocking call to the svm installer for a specified version
fn install_blocking(version: &Version) -> Result<()> {
    tokio::runtime::Runtime::new()
        .unwrap()
        .block_on(svm::install(version))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use ethers::prelude::Lazy;
    use svm::SVM_HOME;

    use super::*;
    use std::{io::Write, str::FromStr};

    #[test]
    fn test_find_installed_version_path() {
        let ver = "0.8.6";
        let version = Version::from_str(ver).unwrap();
        if !svm::installed_versions().unwrap().contains(&version) {
            install_blocking(&version).unwrap();
        }
        let res = find_installed_version_path(&version.to_string()).unwrap();
        let expected = SVM_HOME.join(ver).join(format!("solc-{}", ver));
        assert_eq!(res.unwrap(), expected);
    }

    #[test]
    fn does_not_find_not_installed_version() {
        let ver = "1.1.1";
        let version = Version::from_str(ver).unwrap();
        let res = find_installed_version_path(&version.to_string()).unwrap();
        assert!(res.is_none());
    }

    #[test]
    fn test_find_latest_matching_installation() {
        let mut versions = ["0.4.24", "0.5.1", "0.5.2"]
            .iter()
            .map(|version| Version::from_str(version).unwrap())
            .collect::<Vec<_>>();

        let required = VersionReq::from_str(">=0.4.24").unwrap();

        let got = SolcBuilder::find_matching_installation(&mut versions, &required).unwrap();
        assert_eq!(got, versions[2]);
    }

    #[test]
    fn test_no_matching_installation() {
        let mut versions = ["0.4.24", "0.5.1", "0.5.2"]
            .iter()
            .map(|version| Version::from_str(version).unwrap())
            .collect::<Vec<_>>();

        let required = VersionReq::from_str(">=0.6.0").unwrap();
        let got = SolcBuilder::find_matching_installation(&mut versions, &required);
        assert!(got.is_none());
    }

    // helper for testing solidity file versioning
    struct TempSolidityFile {
        version: String,
        path: PathBuf,
    }

    use std::ops::Deref;

    impl Deref for TempSolidityFile {
        type Target = PathBuf;
        fn deref(&self) -> &PathBuf {
            &self.path
        }
    }

    static TMP_CONTRACTS_DIR: Lazy<PathBuf> = Lazy::new(|| {
        let dir = std::env::temp_dir().join("contracts");
        std::fs::remove_dir_all(&dir).unwrap();
        std::fs::create_dir(&dir).unwrap();
        dir
    });

    impl TempSolidityFile {
        fn new(version: &str) -> Self {
            let path = TMP_CONTRACTS_DIR.join(format!("temp-{}.sol", version));
            let mut file = File::create(&path).unwrap();
            file.write(format!("pragma solidity {};\n", version).as_bytes())
                .unwrap();
            Self {
                path,
                version: version.to_string(),
            }
        }
    }

    #[test]
    fn test_version_req() {
        let versions = ["0.1.2", "^0.5.6", ">=0.7.1", ">0.8.0"];
        let files = versions
            .iter()
            .map(|version| TempSolidityFile::new(version));

        files.for_each(|file| {
            let version_req = SolcBuilder::version_req(&file.path).unwrap();
            assert_eq!(version_req, VersionReq::from_str(&file.version).unwrap());
        });

        // Solidity defines version ranges with a space, whereas the semver package
        // requires them to be separated with a comma
        let version_range = ">=0.8.0 <0.9.0";
        let file = TempSolidityFile::new(version_range);
        let version_req = SolcBuilder::version_req(&file.path).unwrap();
        assert_eq!(version_req, VersionReq::from_str(">=0.8.0,<0.9.0").unwrap());
    }

    #[test]
    // This test might be a bit hard t omaintain
    fn test_detect_version() {
        let mut builder = SolcBuilder::new("", &[], &[]).unwrap();
        for (pragma, expected) in [
            // pinned
            ("=0.4.14", "0.4.14"),
            // Up to later patches
            ("^0.4.14", "0.4.24"),
            // Up to later patches (caret implied)
            ("0.4.14", "0.4.24"),
            // any version above 0.5.0
            (">=0.5.0", "0.8.6"),
            // range
            (">=0.4.0 <0.5.0", "0.4.24"),
        ]
        .iter()
        {
            // println!("Checking {}", pragma);
            let file = TempSolidityFile::new(&pragma);
            let res = builder.detect_version(&file.path).unwrap().unwrap();
            assert_eq!(res.0, Version::from_str(&expected).unwrap());
        }
    }

    #[test]
    // Ensures that the contract versions get correctly assigned to a compiler
    // version given a glob
    fn test_contract_versions() {
        let versions = [
            // pinned
            "=0.4.14",
            // Up to later patches
            "^0.4.14",
            // Up to later patches (caret implied)
            "0.4.14",
            // any version above 0.5.0
            ">=0.5.0",
            // range
            ">=0.4.0 <0.5.0",
        ];
        versions.iter().for_each(|version| {
            TempSolidityFile::new(version);
        });

        let dir = TMP_CONTRACTS_DIR
            .clone()
            .into_os_string()
            .into_string()
            .unwrap();
        let glob = format!("{}/**/*.sol", dir);
        let mut builder = SolcBuilder::new(&glob, &[], &[]).unwrap();

        let versions = builder.contract_versions().unwrap();
        assert_eq!(versions["0.4.14"].len(), 1);
        assert_eq!(versions["0.4.24"].len(), 3);
        assert_eq!(versions["0.8.6"].len(), 1);
    }
}
