#![feature(nll)]

//! A package manager for the Idris language.

extern crate config;
extern crate console;
extern crate crossbeam;
extern crate directories;
#[macro_use]
extern crate failure;
extern crate flate2;
extern crate git2;
extern crate glob;
#[macro_use]
extern crate indexmap;
extern crate indicatif;
extern crate inflector;
extern crate itertools;
#[macro_use]
extern crate nom;
extern crate reqwest;
extern crate scoped_threadpool;
extern crate serde;
#[macro_use]
extern crate serde_derive;
extern crate semver;
extern crate serde_json;
extern crate sha2;
extern crate shell_escape;
#[macro_use]
extern crate slog;
extern crate petgraph;
extern crate symlink;
extern crate tar;
extern crate textwrap;
extern crate toml;
extern crate url;
extern crate url_serde;
extern crate walkdir;

pub mod build;
pub mod cli;
pub mod index;
pub mod package;
pub mod resolve;
pub mod retrieve;
pub mod util;
