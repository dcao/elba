//! Utility functions.

pub mod err;
pub mod lock;

/// Turns an SHA2 hash into a nice hexified string.
pub fn hexify_hash(hash: &[u8]) -> String {
    let mut s = String::new();
    for byte in hash {
        let p = format!("{:02x}", byte);
        s.push_str(&p);
    }
    s
}
