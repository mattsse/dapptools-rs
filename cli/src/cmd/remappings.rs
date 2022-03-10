//! remappings command

use crate::cmd::Cmd;
use clap::{Parser, ValueHint};
use ethers::solc::{
    remappings::{RelativeRemapping, Remapping},
    ProjectPathsConfig,
};
use std::path::{Path, PathBuf};

/// Command to list remappings
#[derive(Debug, Clone, Parser)]
pub struct RemappingArgs {
    #[clap(
        help = "the project's root path, default being the current working directory",
        long,
        value_hint = ValueHint::DirPath
    )]
    root: Option<PathBuf>,
    #[clap(
        help = "the paths where your libraries are installed",
        long,
        value_hint = ValueHint::DirPath
    )]
    lib_paths: Vec<PathBuf>,
}

impl Cmd for RemappingArgs {
    type Output = ();

    fn run(self) -> eyre::Result<Self::Output> {
        let root = self.root.unwrap_or_else(|| std::env::current_dir().unwrap());
        let root = dunce::canonicalize(root)?;

        let lib_paths = if self.lib_paths.is_empty() {
            ProjectPathsConfig::find_libs(&root)
        } else {
            self.lib_paths
        };
        let remappings: Vec<_> =
            lib_paths.iter().flat_map(|lib| relative_remappings(lib, &root)).collect();
        remappings.iter().for_each(|x| println!("{}", x));
        Ok(())
    }
}

/// Returns all remappings found in the `lib` path relative to `root`
pub fn relative_remappings(lib: &Path, root: &Path) -> Vec<Remapping> {
    Remapping::find_many(lib)
        .into_iter()
        .map(|r| RelativeRemapping::new(r, root))
        .map(Into::into)
        .collect()
}
