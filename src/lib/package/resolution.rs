use super::Checksum;
use util::err::{Error, ErrorKind};
use failure::ResultExt;
use flate2::read::GzDecoder;
use reqwest::Client;
use serde::{de, Deserialize, Deserializer, Serialize, Serializer};
use sha2::{Digest, Sha256};
use std::{fmt, fs, path::Path, str::FromStr, io::BufReader};
use symlink::symlink_dir;
use tar::Archive;
use url::Url;
use util::{hexify_hash, lock::DirLock};

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum GitTag {
    Commit(String),
    Tag(String),
}

/// The possible places from which a package can be resolved.
///
/// There are two main sources from which a package can originate: a Direct source (a path or a
/// tarball online or a git repo) and an Index (an indirect source which accrues metadata about
/// Direct sources
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize, Eq, Hash)]
#[serde(untagged)]
pub enum Resolution {
    Direct(DirectRes),
    Index(IndexRes),
    Root,
}

impl From<DirectRes> for Resolution {
    fn from(i: DirectRes) -> Self {
        Resolution::Direct(i)
    }
}

impl From<IndexRes> for Resolution {
    fn from(i: IndexRes) -> Self {
        Resolution::Index(i)
    }
}

impl FromStr for Resolution {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let direct = DirectRes::from_str(s);
        if s == "root" {
            Ok(Resolution::Root)
        } else if direct.is_ok() {
            direct.map(Resolution::Direct)
        } else {
            IndexRes::from_str(s).map(Resolution::Index)
        }
    }
}

impl fmt::Display for Resolution {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Resolution::Direct(d) => write!(f, "{}", d),
            Resolution::Index(i) => write!(f, "{}", i),
            Resolution::Root => write!(f, "root"),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum DirectRes {
    /// Git: the package originated from a git repository.
    Git { repo: Url, tag: GitTag },
    /// Dir: the package is on disk in a folder directory.
    Dir { url: Url },
    /// Tar: the package is an archive stored somewhere.
    ///
    /// Tarballs are the only direct resolution which is allowed to have a checksum; this doesn't
    /// really make sense for DirectRes::Local, and we leave validation of repositories to Git
    /// itself. Checksums are stored in the fragment of the resolution url, with they key being the
    /// checksum format.
    Tar { url: Url, cksum: Option<Checksum> },
}

impl DirectRes {
    pub fn retrieve(&self, client: &Client, target: &DirLock) -> Result<(), Error> {
        match self {
            DirectRes::Tar { url, cksum } => match url.scheme() {
                "http" | "https" => client
                    .get(url.clone())
                    .send()
                    .map_err(|_| Error::from(ErrorKind::CannotDownload))
                    .and_then(|mut r| {
                        let mut buf: Vec<u8> = vec![];
                        r.copy_to(&mut buf).context(ErrorKind::CannotDownload)?;

                        let hash = hexify_hash(Sha256::digest(&buf[..]).as_slice());
                        if let Some(cksum) = cksum {
                            if &cksum.hash == &hash {
                                return Err(ErrorKind::Checksum)?;
                            }
                        }

                        let archive = BufReader::new(&buf[..]);
                        let archive = GzDecoder::new(archive);
                        let mut archive = Archive::new(archive);

                        archive.unpack(target.path()).context(ErrorKind::CannotDownload)?;

                        Ok(())
                    }),
                "file" => {
                    let mut archive = fs::File::open(target.path()).context(ErrorKind::CannotDownload)?;

                    let hash = hexify_hash(
                        Sha256::digest_reader(&mut archive)
                            .context(ErrorKind::CannotDownload)?
                            .as_slice(),
                    );

                    if let Some(cksum) = cksum {
                        if &cksum.hash == &hash {
                            return Err(ErrorKind::Checksum)?;
                        }
                    }

                    let archive = BufReader::new(archive);
                    let archive = GzDecoder::new(archive);
                    let mut archive = Archive::new(archive);

                    archive.unpack(target.path()).context(ErrorKind::CannotDownload)?;

                    Ok(())
                }
                _ => Err(Error::from(ErrorKind::CannotDownload)),
            },
            // TODO: Workspaces.
            DirectRes::Git { repo, tag } => {
                // TODO: What should we do for git repos? Treat repos with different checked out
                // branches/commits as one folder or different ones? If the former, we're going
                // to have to make sure that only one instance of `elba` is running at a time so
                // that multiple copies don't try simultaneously checking out different points
                // of a shared git repo. The latter involves lots n lots n lots of duplication
                unimplemented!()
            },
            DirectRes::Dir { url } => {
                // If this package is located on disk, we just create a symlink into the cache
                // directory.
                let src = url.to_file_path().unwrap();
                // We don't try to copy-paste at all. If we can't symlink, we just give up.
                symlink_dir(src, target.path()).context(ErrorKind::CannotDownload)?;

                Ok(())
            }
        }
    }
}

impl FromStr for DirectRes {
    type Err = Error;

    fn from_str(url: &str) -> Result<Self, Self::Err> {
        let mut parts = url.splitn(2, '+');
        let utype = parts.next().unwrap();
        let url = parts.next().ok_or_else(|| ErrorKind::InvalidSourceUrl)?;

        match utype {
            "git" => unimplemented!(),
            "dir" => {
                let url = Url::parse(url).context(ErrorKind::InvalidSourceUrl)?;
                if url.scheme() != "file" {
                    return Err(ErrorKind::InvalidSourceUrl)?;
                }
                Ok(DirectRes::Dir { url })
            }
            "tar" => {
                let mut url = Url::parse(url).context(ErrorKind::InvalidSourceUrl)?;
                if url.scheme() != "http" || url.scheme() != "https" || url.scheme() != "file" {
                    return Err(ErrorKind::InvalidSourceUrl)?;
                }
                let cksum = url.fragment()
                    .and_then(|x| {
                        Checksum::from_str(x).ok()
                    });
                url.set_fragment(None);
                Ok(DirectRes::Tar { url, cksum })
            }
            _ => Err(ErrorKind::InvalidSourceUrl)?,
        }
    }
}

impl fmt::Display for DirectRes {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            DirectRes::Git {
                repo: _repo,
                tag: _tag,
            } => unimplemented!(),
            DirectRes::Dir { url } => {
                let url = url.as_str();
                write!(f, "dir+{}", url)
            }
            DirectRes::Tar { url, cksum } => {
                let url = url.as_str();
                write!(f, "tar+{}{}", url, if let Some(cksum) = cksum { "#".to_string() + &cksum.to_string() } else { "".to_string() },)
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct IndexRes {
    pub url: Url,
}

impl FromStr for IndexRes {
    type Err = Error;

    fn from_str(url: &str) -> Result<Self, Self::Err> {
        let mut parts = url.splitn(2, '+');
        let utype = parts.next().unwrap();
        let url = parts.next().ok_or_else(|| ErrorKind::InvalidSourceUrl)?;

        match utype {
            "index" => {
                let url = Url::parse(url).context(ErrorKind::InvalidSourceUrl)?;
                Ok(IndexRes { url })
            }
            _ => Err(ErrorKind::InvalidSourceUrl)?,
        }
    }
}

impl fmt::Display for IndexRes {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let url = self.url.as_str();
        let mut s = String::with_capacity(url.len() + 10);
        s.push_str("index+");
        s.push_str(url);
        write!(f, "{}", s)
    }
}

impl Serialize for DirectRes {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for DirectRes {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s = String::deserialize(deserializer)?;
        FromStr::from_str(&s).map_err(de::Error::custom)
    }
}

impl Serialize for IndexRes {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for IndexRes {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s = String::deserialize(deserializer)?;
        FromStr::from_str(&s).map_err(de::Error::custom)
    }
}
