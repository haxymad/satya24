//! Streaming media hashing (one pass, constant memory).

use crate::source::ImageSource;
use md5::Md5;
use sha1::Sha1;
use sha2::{Digest, Sha256};
use std::io;

#[derive(Debug, Clone, serde::Serialize)]
pub struct MediaHashes {
    pub bytes: u64,
    pub md5: String,
    pub sha1: String,
    pub sha256: String,
    pub stored_md5: Option<String>,
    pub stored_sha1: Option<String>,
    /// None when the container stores no hash to compare against.
    pub md5_matches: Option<bool>,
    pub sha1_matches: Option<bool>,
}

/// Hash the whole logical media. `progress(done, total)` is called about
/// every 256 MiB.
pub fn hash_media(src: &dyn ImageSource, mut progress: impl FnMut(u64, u64)) -> io::Result<MediaHashes> {
    let total = src.len();
    let (mut m, mut s1, mut s256) = (Md5::new(), Sha1::new(), Sha256::new());
    let mut buf = vec![0u8; 4 << 20];
    let mut off = 0u64;
    let mut next_report = 0u64;
    while off < total {
        let want = buf.len().min((total - off) as usize);
        src.read_exact_at(off, &mut buf[..want])?;
        m.update(&buf[..want]);
        s1.update(&buf[..want]);
        s256.update(&buf[..want]);
        off += want as u64;
        if off >= next_report {
            progress(off, total);
            next_report = off + (256 << 20);
        }
    }
    let stored = src.stored_hashes();
    let md5 = hex::encode(m.finalize());
    let sha1 = hex::encode(s1.finalize());
    Ok(MediaHashes {
        bytes: off,
        md5_matches: stored.md5.as_ref().map(|s| s.eq_ignore_ascii_case(&md5)),
        sha1_matches: stored.sha1.as_ref().map(|s| s.eq_ignore_ascii_case(&sha1)),
        md5,
        sha1,
        sha256: hex::encode(s256.finalize()),
        stored_md5: stored.md5,
        stored_sha1: stored.sha1,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::source::MemImage;

    #[test]
    fn hashes_known_vector() {
        let h = hash_media(&MemImage(b"abc"), |_, _| {}).unwrap();
        assert_eq!(h.md5, "900150983cd24fb0d6963f7d28e17f72");
        assert_eq!(h.sha256, "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
        assert_eq!(h.md5_matches, None);
    }
}
