//! Positioned, read-only image access.

use std::fs::File;
use std::io;
use std::path::{Path, PathBuf};

/// Hashes recorded inside the evidence container at acquisition time.
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct StoredHashes {
    pub md5: Option<String>,
    pub sha1: Option<String>,
}

/// A read-only evidence image addressed by absolute byte offset.
///
/// Implementations must be safe to share between threads; every read is
/// positioned, so there is no shared cursor.
pub trait ImageSource: Send + Sync {
    /// Logical media size in bytes (for E01: the acquired disk, not the file).
    fn len(&self) -> u64;

    /// Read up to `buf.len()` bytes at `offset`. Returns 0 only at end of media.
    fn read_at(&self, offset: u64, buf: &mut [u8]) -> io::Result<usize>;

    /// Short label for reports, e.g. "e01" or "raw".
    fn kind(&self) -> &'static str;

    /// Hashes stored by the acquisition tool, if the container has any.
    fn stored_hashes(&self) -> StoredHashes {
        StoredHashes::default()
    }

    fn is_empty(&self) -> bool {
        self.len() == 0
    }

    fn read_exact_at(&self, offset: u64, buf: &mut [u8]) -> io::Result<()> {
        let mut done = 0usize;
        while done < buf.len() {
            let n = self.read_at(offset + done as u64, &mut buf[done..])?;
            if n == 0 {
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    format!("read past end of media at offset {}", offset + done as u64),
                ));
            }
            done += n;
        }
        Ok(())
    }

    fn read_vec(&self, offset: u64, len: usize) -> io::Result<Vec<u8>> {
        let mut v = vec![0u8; len];
        self.read_exact_at(offset, &mut v)?;
        Ok(v)
    }
}

/// Raw / dd image (or a block device) read with positioned I/O.
pub struct RawImage {
    file: File,
    len: u64,
}

impl RawImage {
    pub fn open(path: &Path) -> io::Result<Self> {
        use std::io::{Seek, SeekFrom};
        let mut file = File::open(path)?;
        // metadata().len() is 0 for block devices; seeking works for both.
        let len = file.seek(SeekFrom::End(0))?;
        Ok(Self { file, len })
    }
}

impl ImageSource for RawImage {
    fn len(&self) -> u64 {
        self.len
    }

    fn read_at(&self, offset: u64, buf: &mut [u8]) -> io::Result<usize> {
        if offset >= self.len {
            return Ok(0);
        }
        let want = buf.len().min((self.len - offset) as usize);
        #[cfg(unix)]
        {
            use std::os::unix::fs::FileExt;
            self.file.read_at(&mut buf[..want], offset)
        }
        #[cfg(windows)]
        {
            use std::os::windows::fs::FileExt;
            self.file.seek_read(&mut buf[..want], offset)
        }
    }

    fn kind(&self) -> &'static str {
        "raw"
    }
}

/// Expert Witness Format (E01/E02/...) image. Segments are discovered
/// automatically from the path of the first one.
pub struct E01Image {
    reader: ewf::EwfReader,
    pub segments: Vec<PathBuf>,
}

impl E01Image {
    pub fn open(path: &Path) -> io::Result<Self> {
        let reader = ewf::EwfReader::open(path).map_err(io::Error::other)?;
        let segments = discover_segment_names(path);
        Ok(Self { reader, segments })
    }

    pub fn reader(&self) -> &ewf::EwfReader {
        &self.reader
    }
}

impl ImageSource for E01Image {
    fn len(&self) -> u64 {
        self.reader.total_size()
    }

    fn read_at(&self, offset: u64, buf: &mut [u8]) -> io::Result<usize> {
        self.reader.read_at(buf, offset).map_err(io::Error::other)
    }

    fn kind(&self) -> &'static str {
        "e01"
    }

    fn stored_hashes(&self) -> StoredHashes {
        let h = self.reader.stored_hashes();
        StoredHashes {
            md5: h.md5.map(hex::encode),
            sha1: h.sha1.map(hex::encode),
        }
    }
}

/// In-memory image, for tests and for the legacy `&[u8]` code paths.
pub struct MemImage<'a>(pub &'a [u8]);

impl ImageSource for MemImage<'_> {
    fn len(&self) -> u64 {
        self.0.len() as u64
    }

    fn read_at(&self, offset: u64, buf: &mut [u8]) -> io::Result<usize> {
        let len = self.0.len() as u64;
        if offset >= len {
            return Ok(0);
        }
        let start = offset as usize;
        let n = buf.len().min(self.0.len() - start);
        buf[..n].copy_from_slice(&self.0[start..start + n]);
        Ok(n)
    }

    fn kind(&self) -> &'static str {
        "memory"
    }
}

const EVF1_MAGIC: &[u8; 8] = b"EVF\x09\x0d\x0a\xff\x00";
const EVF2_MAGIC: &[u8; 8] = b"EVF2\x0d\x0a\x81\x00";

/// Open an evidence image, choosing the reader from the file's magic bytes
/// (not its extension).
pub fn open_image(path: &Path) -> io::Result<Box<dyn ImageSource>> {
    use std::io::Read;
    let mut magic = [0u8; 8];
    let n = File::open(path)?.read(&mut magic)?;
    if n == 8 && (&magic == EVF1_MAGIC || &magic == EVF2_MAGIC) {
        Ok(Box::new(E01Image::open(path)?))
    } else {
        Ok(Box::new(RawImage::open(path)?))
    }
}

/// List the segment files that sit next to the first segment (for logs only;
/// the ewf crate does its own discovery).
fn discover_segment_names(first: &Path) -> Vec<PathBuf> {
    let mut out = vec![first.to_path_buf()];
    let (Some(dir), Some(stem)) = (first.parent(), first.file_stem()) else {
        return out;
    };
    for i in 2..=99u32 {
        let candidate = dir.join(format!("{}.E{:02}", stem.to_string_lossy(), i));
        if candidate.exists() {
            out.push(candidate);
        } else {
            break;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mem_image_reads_and_clamps() {
        let data: Vec<u8> = (0..100u8).collect();
        let img = MemImage(&data);
        let mut buf = [0u8; 10];
        assert_eq!(img.read_at(95, &mut buf).unwrap(), 5);
        assert_eq!(&buf[..5], &[95, 96, 97, 98, 99]);
        assert_eq!(img.read_at(100, &mut buf).unwrap(), 0);
        assert!(img.read_exact_at(95, &mut buf).is_err());
    }
}
