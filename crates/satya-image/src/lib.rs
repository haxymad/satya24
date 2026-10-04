//! Streaming, read-only access to evidence images.
//!
//! Parsers should depend on [`ImageSource`] instead of `&[u8]`, so a 150 GB
//! E01 never has to fit in RAM. Nothing in this crate opens a file for writing.

pub mod ext;
pub mod fat32;
pub mod hashing;
pub mod partition;
pub mod source;
pub mod vfs;

pub use partition::{probe_fs, read_partitions, FsKind, Partition};
pub use source::{open_image, AcquisitionInfo, E01Image, ImageSource, MemImage, RawImage, SegmentInfo, StoredHashes};
