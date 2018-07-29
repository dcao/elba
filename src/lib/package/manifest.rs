//! Package manifest files.

use self::version::Constraint;
use super::{
    resolution::{DirectRes, IndexRes, Resolution},
    *,
};
use failure::{Error, ResultExt};
use indexmap::IndexMap;
use semver::Version;
use std::{path::PathBuf, str::FromStr};
use toml;
use url::Url;
use url_serde;
use util::errors::*;

// TODO: Package aliasing. Have dummy alias files in the root target folder.
//
// e.g. to alias `me/lightyear` with default root module `Me.Lightyear` as the module
// `Yeet.Lightyeet`, in the target folder, we make the following file in the proper directory
// (directory won't matter for Blodwen/Idris 2):
//
// ```idris
// module Yeet.Lightyeet
//
// import public Me.Lightyear
// ```
//
// Behind the scenes, we build this as its own package with the package it's aliasing as
// its only dependency, throw it in the global cache, and add this to the import dir of the root
// package instead of the original.
//
// I guess this also means that each package should declare their (root) module(s), so that we
// can identify conflicts ahead of time without having to guess that it's always gonna be Group.Name
//
// With this in place, we can safely avoid module namespace conflicts.

#[derive(Deserialize, Debug, Clone)]
pub struct Manifest {
    pub package: PackageInfo,
    #[serde(default = "IndexMap::new")]
    pub dependencies: IndexMap<Name, DepReq>,
    #[serde(default = "IndexMap::new")]
    pub dev_dependencies: IndexMap<Name, DepReq>,
    pub targets: Targets,
    #[serde(default)]
    workspace: IndexMap<Name, String>,
}

impl Manifest {
    pub fn summary(&self) -> Summary {
        let pid = PackageId::new(self.package.name.clone(), Resolution::Root);
        Summary::new(pid, self.package.version.clone())
    }

    pub fn version(&self) -> &Version {
        &self.package.version
    }

    pub fn deps(&self, def_index: &IndexRes, dev_deps: bool) -> IndexMap<PackageId, Constraint> {
        let mut deps = indexmap!();
        for (n, dep) in &self.dependencies {
            let dep = dep.clone();
            let (pid, c) = dep.into_dep(def_index.clone(), n.clone());
            deps.insert(pid, c);
        }

        if dev_deps {
            for (n, dep) in &self.dev_dependencies {
                let dep = dep.clone();
                let (pid, c) = dep.into_dep(def_index.clone(), n.clone());
                deps.insert(pid, c);
            }
        }

        deps
    }
}

impl FromStr for Manifest {
    type Err = Error;

    fn from_str(raw: &str) -> Result<Self, Self::Err> {
        let toml: Manifest = toml::from_str(raw)
            .context(ErrorKind::InvalidManifestFile)
            .map_err(Error::from)?;

        if toml.targets.lib.is_none() && toml.targets.bin.is_empty() {
            bail!("manifests must define at least either a bin or lib target")
        } else {
            Ok(toml)
        }
    }
}

#[derive(Deserialize, Debug, Clone)]
pub struct PackageInfo {
    pub name: Name,
    pub version: Version,
    pub authors: Vec<String>,
    pub license: Option<String>,
}

#[derive(Deserialize, Debug, Clone)]
#[serde(untagged)]
pub enum DepReq {
    Registry(Constraint),
    RegLong {
        con: Constraint,
        registry: IndexRes,
    },
    Local {
        path: PathBuf,
    },
    Git {
        #[serde(with = "url_serde")]
        git: Url,
        #[serde(default = "default_tag")]
        tag: String,
    },
}

fn default_tag() -> String {
    "master".to_owned()
}

impl DepReq {
    pub fn into_dep(self, def_index: IndexRes, n: Name) -> (PackageId, Constraint) {
        match self {
            DepReq::Registry(c) => {
                let pi = PackageId::new(n, def_index.into());
                (pi, c)
            }
            DepReq::RegLong { con, registry } => {
                let pi = PackageId::new(n, registry.into());
                (pi, con)
            }
            DepReq::Local { path } => {
                let res = DirectRes::Dir { url: path };
                let pi = PackageId::new(n, res.into());
                (pi, Constraint::any())
            }
            DepReq::Git { git, tag } => {
                let res = DirectRes::Git { repo: git, tag };
                let pi = PackageId::new(n, res.into());
                (pi, Constraint::any())
            }
        }
    }
}

#[derive(Deserialize, Debug, Clone)]
pub struct Targets {
    pub lib: Option<LibTarget>,
    #[serde(default = "Vec::new")]
    bin: Vec<BinTarget>,
    #[serde(default = "Vec::new")]
    test: Vec<Target>,
    #[serde(default = "Vec::new")]
    bench: Vec<Target>,
}

#[derive(Deserialize, Debug, Clone)]
pub struct Target {
    path: PathBuf,
}

#[derive(Deserialize, Debug, Clone)]
pub struct LibTarget {
    pub path: PathBuf,
    pub mods: Vec<String>,
}

#[derive(Deserialize, Debug, Clone)]
pub struct BinTarget {
    name: String,
    // For binaries, benches, and tests, this should point to a file with a Main module.
    main: PathBuf,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valid_manifest() {
        let manifest = r#"
[package]
name = 'ring_ding/test'
version = '1.0.0'
authors = ['me']
license = 'MIT'

[dependencies]
'awesome/a' = '>= 1.0.0 < 2.0.0'
'cool/b' = { git = 'https://github.com/super/cool', tag = "v1.0.0" }
'great/c' = { path = 'here/right/now' }

[dev_dependencies]
'ayy/x' = '2.0'

[[targets.bin]]
name = 'bin1'
main = 'src/bin/Here.idr'

[targets.lib]
path = "src/lib/"
mods = [
    "Control.Monad.Wow",
    "Control.Monad.Yeet",
    "RingDing.Test"
]
"#;

        assert!(Manifest::from_str(manifest).is_ok());
    }
}
