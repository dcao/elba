//! Interfaces for retrieving packages (and information about them) from different sources.
//!
//! Packages can originate from several sources, which complicates getting metadata about them.
//! This module is responsible for smoothing over that process, as well as coordinating the actual
//! retrieval of packages from various different sources (hopefully in parallel).

mod cache;

pub use self::cache::{Binary, BuildHash, Cache, Source};
use failure::{Error, ResultExt};
use index::Indices;
use package::{
    resolution::{DirectRes, IndexRes, Resolution},
    version::{Constraint, Interval, Range, Relation},
    PackageId, Summary,
};
use resolve::incompat::{Incompatibility, IncompatibilityCause};
use semver::Version;
use slog::Logger;
use std::env;
use util::errors::{ErrorKind, Res};
use util::graph::Graph;

// TODO: Patching
// TODO: Multiple root packages so we can support workspaces
/// Retrieves the best packages using both the indices available and a lockfile.
/// By default, prioritizes using a lockfile.
#[derive(Debug)]
pub struct Retriever<'cache> {
    /// The local cache of packages.
    cache: &'cache Cache,
    root: Summary,
    root_deps: Vec<(PackageId, Constraint)>,
    indices: Indices,
    lockfile: Graph<Summary>,
    pub logger: Logger,
    pub def_index: IndexRes,
}

impl<'cache> Retriever<'cache> {
    pub fn new(
        plog: &Logger,
        cache: &'cache Cache,
        root: Summary,
        root_deps: Vec<(PackageId, Constraint)>,
        indices: Indices,
        lockfile: Graph<Summary>,
        def_index: IndexRes,
    ) -> Self {
        let logger = plog.new(o!("root" => root.to_string()));

        Retriever {
            cache,
            root,
            root_deps,
            indices,
            lockfile,
            logger,
            def_index,
        }
    }

    /// Loads all of the packages selected in a Solve into the Cache, returning a new graph of all
    /// the Sources.
    ///
    /// This downloads all the packages into the cache. If we wanted to parallelize downloads
    /// later, this is where we'd deal with all the Tokio stuff.
    pub fn retrieve_packages(&mut self, solve: &Graph<Summary>) -> Res<Graph<Source>> {
        let sources = solve.map(
            |_, sum| {
                let wd = DirectRes::Dir {
                    url: env::current_dir()?,
                };

                let loc = match sum.resolution() {
                    Resolution::Direct(direct) => direct,
                    Resolution::Root => &wd,
                    Resolution::Index(_) => &self.indices.select(sum).unwrap().location,
                };

                let source = self
                    .cache
                    .checkout_source(sum.id(), loc, Some(sum.version()))
                    .context(format_err!("unable to retrieve package {}", sum))?;

                Ok(source)
            },
            |_| Ok(()),
        )?;

        Ok(sources)
    }

    /// Chooses the best version of a package given a constraint.
    pub fn best(
        &mut self,
        pkg: &PackageId,
        con: &Constraint,
        minimize: bool,
    ) -> Result<Version, Error> {
        // With stuff from lockfiles, we try to retrieve whatever version was specified in the
        // lockfile. However, if it fails, we don't want to error out; we want to try to find
        // the best version we can otherwise.
        let pkg_verion = self
            .lockfile
            .find_by(|sum| sum.id == *pkg)
            .map(|meta| &meta.version);
        if let Some(v) = pkg_verion {
            if con.satisfies(&v) {
                let dir = if let Resolution::Direct(loc) = pkg.resolution() {
                    Some(loc)
                } else {
                    self.indices
                        .select(&Summary::new(pkg.clone(), v.clone()))
                        .map(|e| &e.location)
                        .ok()
                };

                if let Some(dir) = dir {
                    if let Ok(src) = self.cache.checkout_source(pkg, dir, Some(&v)) {
                        return Ok(src.meta().version().clone());
                    }
                }
            }
        }

        if let Resolution::Direct(loc) = pkg.resolution() {
            return Ok(self
                .cache
                .checkout_source(pkg, loc, None)?
                .meta()
                .version()
                .clone());
        }

        if let Resolution::Root = pkg.resolution() {
            return Ok(self.root.version.clone());
        }

        let (mut pre, mut not_pre): (Vec<Version>, Vec<Version>) = self
            .indices
            .entries(pkg)?
            .clone()
            .into_iter()
            .map(|v| v.0)
            .filter(|v| con.satisfies(v))
            .partition(|v| v.is_prerelease());

        if !not_pre.is_empty() {
            if !minimize {
                Ok(not_pre.pop().unwrap())
            } else {
                Ok(not_pre.remove(0))
            }
        } else if !pre.is_empty() {
            if !minimize {
                Ok(pre.pop().unwrap())
            } else {
                Ok(pre.remove(0))
            }
        } else {
            Err(Error::from(ErrorKind::PackageNotFound))
        }
    }

    // TODO: Incompat cache
    /// Returns a `Vec<Incompatibility>` corresponding to the package's dependencies.
    pub fn incompats(&mut self, pkg: &Summary) -> Result<Vec<Incompatibility>, Error> {
        if pkg == &self.root {
            let mut res = vec![];
            for dep in &self.root_deps {
                res.push(Incompatibility::from_dep(
                    pkg.clone(),
                    (dep.0.clone(), dep.1.complement()),
                ));
            }
            return Ok(res);
        }

        // If this is a DirectRes dep, we ask the cache for info.
        if let Resolution::Direct(loc) = pkg.resolution() {
            let deps = self
                .cache
                .checkout_source(pkg.id(), loc, Some(pkg.version()))?
                .meta()
                .deps(&self.def_index, false);
            let mut res = vec![];
            for dep in deps {
                res.push(Incompatibility::from_dep(
                    pkg.clone(),
                    (dep.0.clone(), dep.1.complement()),
                ));
            }
            return Ok(res);
        }

        let entries = self.indices.entries(pkg.id())?;

        let l = entries.len();

        let (ix, ver, start_deps) = entries
            .get_full(pkg.version())
            .map(|x| (x.0, x.1, &x.2.dependencies))
            .ok_or_else(|| ErrorKind::PackageNotFound)?;
        let mut res = vec![];

        for dep in start_deps {
            let mut lix = ix;
            let mut lower = ver;
            let mut rix = ix;
            let mut upper = ver;

            while lix > 0 {
                lix -= 1;
                let new = entries.get_index(lix).unwrap();
                let new_deps = &new.1.dependencies;
                let mut seen = false;
                for new_dep in new_deps {
                    if dep.name == new_dep.name && dep.index == new_dep.index {
                        let rel = dep.req.relation(&new_dep.req);
                        if rel == Relation::Equal || rel == Relation::Superset {
                            seen = true;
                            lower = &new.0;
                        } else {
                            seen = false;
                        }
                    }
                }
                if !seen {
                    lix += 1;
                    break;
                }
            }

            while rix < l - 1 {
                rix += 1;
                let new = entries.get_index(rix).unwrap();
                let new_deps = &new.1.dependencies;
                let mut seen = false;
                for new_dep in new_deps {
                    if dep.name == new_dep.name && dep.index == new_dep.index {
                        let rel = dep.req.relation(&new_dep.req);
                        if rel == Relation::Equal || rel == Relation::Superset {
                            seen = true;
                            upper = &new.0;
                        } else {
                            seen = false;
                        }
                    }
                }
                if !seen {
                    rix -= 1;
                    break;
                }
            }

            let nl = if lix == 0 && rix == l - 1 {
                Interval::Unbounded
            } else {
                Interval::Closed(lower.clone(), false)
            };

            let nu = if lix == 0 && rix == l - 1 {
                Interval::Unbounded
            } else {
                Interval::Closed(upper.clone(), false)
            };

            let dep_pkg = PackageId::new(dep.name.clone(), dep.index.clone().into());

            let cs = indexmap!(
                pkg.id().clone() => Range::new(nl, nu).unwrap().into(),
                dep_pkg => dep.req.complement(),
            );

            res.push(Incompatibility::new(cs, IncompatibilityCause::Dependency))
        }

        Ok(res)
    }

    pub fn count_versions(&self, pkg: &PackageId) -> usize {
        self.indices.count_versions(pkg)
    }

    pub fn root(&self) -> &Summary {
        &self.root
    }
}
