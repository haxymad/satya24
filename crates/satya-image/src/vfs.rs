//! One browsing interface over the filesystems inside an image.
//!
//! Nodes are addressed by stable ids instead of paths, so deleted entries
//! (which may share names with live ones) can be opened too:
//!   "root"        volume root
//!   "c<cluster>"  FAT32 directory starting at <cluster>
//!   "e<offset>"   FAT32 file whose 32-byte directory entry is at <offset>
//!   "i<inode>"    ext inode

use crate::ext::{ExtFs, Inode, ROOT_INODE};
use crate::fat32::{Extent, Fat32, FatEntry};
use crate::partition::{FsKind, Partition};
use crate::source::ImageSource;
use std::io;

pub enum Volume<'a> {
    Fat(Fat32<'a>),
    Ext(ExtFs<'a>),
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct VfsEntry {
    pub name: String,
    pub node: String,
    pub is_dir: bool,
    pub size: u64,
    pub deleted: bool,
    /// False when the content cannot be read back reliably (e.g. ext3
    /// zeroed the block map, or a deleted FAT file's clusters were assumed).
    pub recoverable: bool,
    pub created: Option<String>,
    pub modified: Option<String>,
    pub accessed: Option<String>,
    pub changed: Option<String>,
    pub deleted_at: Option<String>,
    /// "device local (no timezone)" for FAT, "UTC" for ext.
    pub time_basis: &'static str,
    /// Cluster or inode number, for the examiner's notes.
    pub id: String,
    /// Absolute byte offset of the first data byte on the disk.
    pub disk_offset: Option<u64>,
    pub note: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct VolumeInfo {
    pub fs: &'static str,
    pub label: String,
    pub cluster_or_block_size: u64,
    pub details: Vec<(String, String)>,
}

/// Partitions to browse: the partition table, or the whole image when it is
/// a bare filesystem (no table).
pub fn volumes(src: &dyn ImageSource) -> Vec<Partition> {
    let parts = crate::partition::read_partitions(src).unwrap_or_default();
    if !parts.is_empty() {
        return parts;
    }
    let fs = crate::partition::probe_fs(src, 0);
    if fs == FsKind::Unknown || src.len() < 1024 {
        return Vec::new();
    }
    vec![Partition {
        index: 0,
        scheme: "none",
        start_lba: 0,
        end_lba: src.len() / 512 - 1,
        type_id: String::new(),
        name: "whole image".into(),
        fs,
    }]
}

impl<'a> Volume<'a> {
    pub fn open(src: &'a dyn ImageSource, part: &Partition) -> io::Result<Self> {
        match part.fs {
            FsKind::Fat32 => Ok(Volume::Fat(Fat32::open(src, part.offset())?)),
            FsKind::Ext2 | FsKind::Ext3 | FsKind::Ext4 => Ok(Volume::Ext(ExtFs::open(src, part.offset())?)),
            other => Err(io::Error::new(io::ErrorKind::Unsupported, format!("browsing {} is not supported yet", other.as_str()))),
        }
    }

    pub fn info(&self) -> VolumeInfo {
        match self {
            Volume::Fat(f) => VolumeInfo {
                fs: "FAT32",
                label: f.volume_label.clone(),
                cluster_or_block_size: f.cluster_size,
                details: vec![
                    ("Volume ID".into(), format!("{:08X}", f.volume_id)),
                    ("Clusters".into(), f.cluster_count.to_string()),
                    ("Cluster size".into(), format!("{} bytes", f.cluster_size)),
                    ("FAT offset".into(), f.fat_offset.to_string()),
                    ("Data offset".into(), f.data_offset.to_string()),
                ],
            },
            Volume::Ext(e) => VolumeInfo {
                fs: "ext",
                label: e.volume_name.clone(),
                cluster_or_block_size: e.block_size,
                details: vec![
                    ("UUID".into(), e.uuid.clone()),
                    ("Block size".into(), format!("{} bytes", e.block_size)),
                    ("Inodes".into(), e.inodes_count.to_string()),
                    ("Created (UTC)".into(), fmt_utc(e.created).unwrap_or_else(|| "-".into())),
                    ("Last mount (UTC)".into(), fmt_utc(e.last_mount).unwrap_or_else(|| "-".into())),
                    ("Last write (UTC)".into(), fmt_utc(e.last_write).unwrap_or_else(|| "-".into())),
                    ("Last mounted on".into(), if e.last_mounted.is_empty() { "-".into() } else { e.last_mounted.clone() }),
                ],
            },
        }
    }

    pub fn list(&self, node: &str) -> io::Result<Vec<VfsEntry>> {
        let mut out = match self {
            Volume::Fat(f) => {
                let entries = match node {
                    "root" => f.list_root()?,
                    n if n.starts_with('c') => f.list_at(parse(&n[1..])? as u32)?,
                    _ => return Err(bad_node(node)),
                };
                entries.iter().map(|e| fat_entry(f, e)).collect::<Vec<_>>()
            }
            Volume::Ext(x) => {
                let dir = x.inode(match node {
                    "root" => ROOT_INODE,
                    n if n.starts_with('i') => parse(&n[1..])? as u32,
                    _ => return Err(bad_node(node)),
                })?;
                x.list(&dir)?
                    .into_iter()
                    .map(|d| ext_entry(x, &d.name, d.inode, d.deleted, d.file_type))
                    .collect()
            }
        };
        out.sort_by(|a, b| b.is_dir.cmp(&a.is_dir).then(a.deleted.cmp(&b.deleted)).then(a.name.to_lowercase().cmp(&b.name.to_lowercase())));
        Ok(out)
    }

    /// Size and disk extents of a file node.
    pub fn extents(&self, node: &str) -> io::Result<(u64, Vec<Extent>)> {
        match self {
            Volume::Fat(f) => {
                let e = self.fat_file(f, node)?;
                let map = f.file_map(&e);
                Ok((e.size as u64, map.extents))
            }
            Volume::Ext(x) => {
                let ino = x.inode(ext_node(node)?)?;
                Ok((ino.size, x.extents(&ino)?))
            }
        }
    }

    pub fn read(&self, node: &str, offset: u64, len: usize) -> io::Result<Vec<u8>> {
        match self {
            Volume::Fat(f) => {
                let e = self.fat_file(f, node)?;
                let map = f.file_map(&e);
                let end = (offset + len as u64).min(e.size as u64);
                if offset >= end {
                    return Ok(Vec::new());
                }
                let mut buf = vec![0u8; (end - offset) as usize];
                let n = f.read_at(&map, offset, &mut buf)?;
                buf.truncate(n);
                Ok(buf)
            }
            Volume::Ext(x) => {
                let ino = x.inode(ext_node(node)?)?;
                x.read(&ino, offset, len)
            }
        }
    }

    fn fat_file(&self, f: &Fat32, node: &str) -> io::Result<FatEntry> {
        let off = node.strip_prefix('e').ok_or_else(|| bad_node(node))?;
        let e = f.entry_at(parse(off)?)?;
        if e.is_dir {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "node is a directory"));
        }
        Ok(e)
    }
}

fn fat_entry(f: &Fat32, e: &FatEntry) -> VfsEntry {
    let map = (!e.is_dir).then(|| f.file_map(e));
    let disk_offset = if e.first_cluster >= 2 { Some(f.cluster_offset(e.first_cluster)) } else { None };
    let fmt = |t: Option<chrono::NaiveDateTime>| t.map(|t| t.format("%Y-%m-%d %H:%M:%S").to_string());
    VfsEntry {
        name: e.name.clone(),
        node: if e.is_dir { format!("c{}", e.first_cluster) } else { format!("e{}", e.dirent_offset) },
        is_dir: e.is_dir,
        size: e.size as u64,
        deleted: e.deleted,
        recoverable: !e.deleted || e.size == 0 || e.first_cluster >= 2,
        created: fmt(e.created),
        modified: fmt(e.modified),
        accessed: e.accessed.map(|d| d.to_string()),
        changed: None,
        deleted_at: None,
        time_basis: "device local (no timezone)",
        id: format!("cluster {}", e.first_cluster),
        disk_offset,
        note: match (&map, e.deleted) {
            (_, true) => Some("deleted: first character of the name is lost; content assumes contiguous clusters".into()),
            (Some(m), false) if !m.complete && e.size > 0 => Some("cluster chain shorter than the file size".into()),
            _ => None,
        },
    }
}

fn ext_entry(x: &ExtFs, name: &str, inode: u32, deleted: bool, ftype: u8) -> VfsEntry {
    let ino: Option<Inode> = (inode != 0).then(|| x.inode(inode).ok()).flatten();
    let is_dir = match &ino {
        Some(i) if !deleted || i.dtime.is_none() => i.is_dir(),
        _ => ftype == 2,
    };
    let reused = deleted && ino.as_ref().is_some_and(|i| i.links > 0 && i.dtime.is_none());
    let recoverable = !deleted || (!reused && ino.as_ref().is_some_and(|i| i.has_blocks() && i.size > 0));
    let disk_offset = ino.as_ref().and_then(|i| x.extents(i).ok()).and_then(|v| v.first().map(|e| e.disk_offset));
    let note = if inode == 0 {
        Some("deleted: inode number was cleared".to_string())
    } else if reused {
        Some(format!("deleted: inode {inode} now belongs to another file"))
    } else if deleted && !recoverable {
        Some("deleted: name and times survive, but ext3 zeroed the block map, so content is not recoverable from the inode".to_string())
    } else if deleted {
        Some("deleted: content read through the surviving block map".to_string())
    } else {
        None
    };
    VfsEntry {
        name: name.to_string(),
        node: format!("i{inode}"),
        is_dir,
        size: if reused { 0 } else { ino.as_ref().map(|i| i.size).unwrap_or(0) },
        deleted,
        recoverable,
        created: None,
        modified: ino.as_ref().and_then(|i| fmt_utc(i.mtime)),
        accessed: ino.as_ref().and_then(|i| fmt_utc(i.atime)),
        changed: ino.as_ref().and_then(|i| fmt_utc(i.ctime)),
        deleted_at: if reused { None } else { ino.as_ref().and_then(|i| fmt_utc(i.dtime)) },
        time_basis: "UTC",
        id: format!("inode {inode}"),
        disk_offset: if reused { None } else { disk_offset },
        note,
    }
}

fn fmt_utc(t: Option<chrono::DateTime<chrono::Utc>>) -> Option<String> {
    t.map(|t| t.format("%Y-%m-%d %H:%M:%S").to_string())
}

fn ext_node(node: &str) -> io::Result<u32> {
    if node == "root" {
        return Ok(ROOT_INODE);
    }
    Ok(parse(node.strip_prefix('i').ok_or_else(|| bad_node(node))?)? as u32)
}

fn parse(s: &str) -> io::Result<u64> {
    s.parse().map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, format!("bad node id: {s}")))
}

fn bad_node(n: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, format!("unknown node: {n}"))
}
