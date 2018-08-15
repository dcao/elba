//! Package manifest files.

use self::version::Constraint;
use super::{
    resolution::{DirectRes, IndexRes},
    *,
};
use failure::{Error, ResultExt};
use indexmap::IndexMap;
use semver::Version;
use std::{
    path::{Path, PathBuf},
    str::FromStr,
};
use toml;
use url::Url;
use url_serde;
use util::SubPath;

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
    pub workspace: IndexMap<Name, SubPath>,
}

impl Manifest {
    // Returns only the workspace portion of a manifest.
    pub fn workspace(s: &str) -> Option<IndexMap<Name, SubPath>> {
        toml::value::Value::try_from(&s)
            .ok()?
            .get("workspace")?
            .clone()
            .try_into()
            .ok()
    }

    pub fn version(&self) -> &Version {
        &self.package.version
    }

    pub fn name(&self) -> &Name {
        &self.package.name
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
            .with_context(|e| format_err!("invalid manifest file: {}", e))
            .map_err(Error::from)?;

        if toml.targets.lib.is_none() && toml.targets.bin.is_empty() {
            bail!("manifests must define at least either a bin or lib target")
        }

        Ok(toml)
    }
}

#[derive(Deserialize, Debug, Clone)]
pub struct PackageInfo {
    pub name: Name,
    pub version: Version,
    pub authors: Vec<String>,
    pub description: Option<String>,
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
                let res = DirectRes::Dir { path };
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
    pub bin: Vec<BinTarget>,
    #[serde(default = "Vec::new")]
    pub test: Vec<TestTarget>,
}

#[derive(Deserialize, Debug, Clone)]
pub struct LibTarget {
    #[serde(default = "default_lib_subpath")]
    pub path: SubPath,
    pub mods: Vec<String>,
    #[serde(default)]
    pub idris_opts: Vec<String>,
}

fn default_lib_subpath() -> SubPath {
    SubPath::from_path(Path::new("src")).unwrap()
}

#[derive(Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct BinTarget {
    pub name: String,
    #[serde(default = "default_bin_subpath")]
    pub path: SubPath,
    pub main: String,
    #[serde(default)]
    pub idris_opts: Vec<String>,
}

fn default_bin_subpath() -> SubPath {
    SubPath::from_path(Path::new("src")).unwrap()
}

/// A TestTarget is literally exactly the same as a BinTarget, with the only difference being
/// the difference in default path.
///
/// I know, code duplication sucks and is stupid, but what can ya do :v
#[derive(Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct TestTarget {
    pub name: Option<String>,
    #[serde(default = "default_test_subpath")]
    pub path: SubPath,
    pub main: String,
    #[serde(default)]
    pub idris_opts: Vec<String>,
}

fn default_test_subpath() -> SubPath {
    SubPath::from_path(Path::new("tests")).unwrap()
}

impl From<TestTarget> for BinTarget {
    fn from(t: TestTarget) -> Self {
        let default_name = format!("tests-{}", &t.main)
            .trim_right_matches(".idr")
            .replace("/", "_")
            .replace(".", "_");

        BinTarget {
            name: t.name.unwrap_or(default_name),
            path: t.path,
            main: t.main,
            idris_opts: t.idris_opts,
        }
    }
}

impl BinTarget {
    // A note on extensions:
    // - If the extension of the target_path is idr or empty, it will be treated as a Main file.
    // - If the extension of the target_path is anything else, that extension will be the function
    //   of the preceding part's module which will be treated as the main function.
    pub fn resolve_bin(&self, parent: &Path) -> Option<(PathBuf, PathBuf)> {
        let main_path: PathBuf = self.main.clone().into();
        // If the main path is a valid SubPath, we just use that.
        if let Ok(s) = SubPath::from_path(&main_path) {
            if parent.join(&s.0).with_extension("idr").exists() {
                let target_path = if s.0.extension().is_none() {
                    parent.join(&s.0).with_extension("idr")
                } else {
                    parent.join(&s.0)
                };
                let src_path = target_path.parent().unwrap();
                // This is the relative target path
                let target_path: PathBuf = target_path.file_name().unwrap().to_os_string().into();
                return Some((src_path.to_path_buf(), target_path));
            }
        }

        // Otherwise, we have to do more complicated logic.
        let src_path = parent.join(&self.path.0);
        let mut split = self.main.trim_matches('.').rsplitn(2, '.');
        let (after, before) = (split.next().unwrap(), split.next());

        if let Some(before) = before {
            let target_path: PathBuf = before.replace(".", "/").into();
            // If there is at least one dot in the name:
            if src_path
                .join(&target_path)
                .join(after)
                .with_extension("idr")
                .exists()
            {
                // If a file corresponding to the whole module name exists, we use that.
                Some((src_path, target_path.join(after).with_extension("idr")))
            } else if src_path.join(&target_path).with_extension("idr").exists() {
                // Otherwise, if a file corresponding to the module name minus the last
                // part exists, we assume that the last part refers to a function which
                // should be treated as the main function.
                Some((src_path, target_path.with_extension(after)))
            } else {
                None
            }
        } else {
            let target_path: PathBuf = after.into();
            // Otherwise, if the name has no dots:
            if src_path.join(&target_path).with_extension("idr").exists() {
                Some((src_path, target_path.with_extension("idr")))
            } else {
                None
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifest_valid() {
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
main = 'src/bin/Here'

[targets.lib]
path = "src/lib/"
mods = [
    "Control.Monad.Wow",
    "Control.Monad.Yeet",
    "RingDing.Test"
]
idris_opts = ["--warnpartial", "--warnreach"]
"#;

        assert!(Manifest::from_str(manifest).is_ok());
    }

    #[test]
    fn manifest_no_targets() {
        let manifest = r#"
[package]
name = 'ring_ding/test'
version = '1.0.0'
authors = ['Me <y@boi.me>']
license = 'MIT'

[dependencies]
'awesome/a' = '>= 1.0.0 < 2.0.0'
'cool/b' = { git = 'https://github.com/super/cool', branch = "v1.0.0" }
'great/c' = { path = 'here/right/now' }

[dev_dependencies]
'ayy/x' = '2.0'
"#;

        assert!(Manifest::from_str(manifest).is_err());
    }

    #[test]
    fn manifest_invalid_target_path() {
        let manifest = r#"
[package]
name = 'ring_ding/test'
version = '1.0.0'
description = "a cool package"
authors = ['me']
license = 'MIT'

[dependencies]
'awesome/a' = '>= 1.0.0 < 2.0.0'
'cool/b' = { git = 'https://github.com/super/cool', tag = "v1.0.0" }
'great/c' = { path = 'here/right/now' }

[dev_dependencies]
'ayy/x' = '2.0'

[targets.lib]
path = "../oops"
mods = [
    "Right.Here"
]
"#;

        assert!(Manifest::from_str(manifest).is_err());
    }
}
