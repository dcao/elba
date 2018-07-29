//! Module `package/lockfile` contains logic for (de)serializing lockfiles.
//!
//! Lockfiles are created based on dependency constraints, and ensure that builds are repeatable

use failure::{Error, ResultExt};
use indexmap::IndexSet;
use petgraph;
use std::iter::FromIterator;
use toml;
use util::graph::Graph;

use super::*;

#[derive(Clone, Deserialize, Debug, Serialize)]
pub struct LockfileToml {
    pub packages: IndexSet<LockedPkg>,
}

#[derive(Clone, Deserialize, Debug, Serialize, PartialEq, Eq, Hash)]
pub struct LockedPkg {
    #[serde(flatten)]
    pub sum: Summary,
    #[serde(default = "Vec::new")]
    pub dependencies: Vec<Summary>,
}

impl FromStr for LockfileToml {
    type Err = Error;

    fn from_str(raw: &str) -> Result<Self, Self::Err> {
        toml::from_str(raw)
            .context(ErrorKind::InvalidLockfile)
            .map_err(Error::from)
    }
}

// TODO: Is here a good place for these?
// TODO: impl Into<Graph> for LockfileToml
// Should the package module unaware of resolve module?
impl Into<LockfileToml> for Graph<Summary> {
    fn into(self) -> LockfileToml {
        // TODO: Is the unwrap safe here?
        let root = self
            .find_by(|sum| *sum.resolution() == Resolution::Root)
            .unwrap();

        let pkg_iter = self
            .sub_tree(self.find_id(root).unwrap())
            .map(|(_, pkg)| LockedPkg {
                sum: pkg.clone(),
                dependencies: self
                    .children(self.find_id(pkg).unwrap())
                    .map(|x| x.1)
                    .cloned()
                    .collect(),
            });

        let packages = IndexSet::from_iter(pkg_iter);
        LockfileToml { packages }
    }
}

// TODO: verify that this is a valid solve
impl From<LockfileToml> for Graph<Summary> {
    fn from(f: LockfileToml) -> Self {
        let mut tree = petgraph::Graph::new();
        let mut set = indexmap!();

        // We don't assume that nix 0 is root here.
        for pkg in f.packages {
            let nix = if set.contains_key(&pkg.sum) {
                set[&pkg.sum]
            } else {
                let nix = tree.add_node(pkg.sum.clone());
                set.insert(pkg.sum, nix);
                nix
            };

            for dep in pkg.dependencies {
                let dep_nix = if set.contains_key(&dep) {
                    set[&dep]
                } else {
                    let nix = tree.add_node(dep.clone());
                    set.insert(dep, nix);
                    nix
                };

                tree.add_edge(nix, dep_nix, ());
            }
        }

        Graph::new(tree)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valid_lockfile() {
        let lockfile = r#"
[[packages]]
id = "terminator/one@index+tar+https://elba.io/pkg"
version = "0.1.4"

[[packages]]
id = "good/package@dir+/here/there"
version = "1.0.5-alpha.5-zeta.15"
dependencies = [
    { id = "terminator/one@index+tar+https://elba.io/pkg", version = "0.1.4" }
]
        "#;

        assert!(LockfileToml::from_str(lockfile).is_ok());
    }
}
