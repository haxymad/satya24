//! Read-only ext2/ext3/ext4 reader: superblock, inodes, directories (with
//! deleted names recovered from directory slack), block maps and extents.
//!
//! Deleted files: ext3 zeroes an inode's block map when a file is deleted,
//! so names and times usually survive but contents do not. Entries say so
//! (`recoverable == false`) instead of guessing.

use crate::fat32::Extent;
use crate::source::ImageSource;
use chrono::{DateTime, Utc};
use std::io;

const MAGIC: u16 = 0xEF53;
const INCOMPAT_FILETYPE: u32 = 0x2;
const INCOMPAT_64BIT: u32 = 0x80;
const EXTENTS_FL: u32 = 0x80000;
const EXT_MAGIC: u16 = 0xF30A;
pub const ROOT_INODE: u32 = 2;

pub struct ExtFs<'a> {
    src: &'a dyn ImageSource,
    pub base: u64,
    pub block_size: u64,
    pub inodes_count: u32,
    pub inodes_per_group: u32,
    pub inode_size: u64,
    pub volume_name: String,
    pub last_mounted: String,
    pub uuid: String,
    pub created: Option<DateTime<Utc>>,
    pub last_mount: Option<DateTime<Utc>>,
    pub last_write: Option<DateTime<Utc>>,
    filetype: bool,
    inode_tables: Vec<u64>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct Inode {
    pub number: u32,
    pub mode: u16,
    pub size: u64,
    pub links: u16,
    pub flags: u32,
    pub atime: Option<DateTime<Utc>>,
    pub ctime: Option<DateTime<Utc>>,
    pub mtime: Option<DateTime<Utc>>,
    pub dtime: Option<DateTime<Utc>>,
    #[serde(skip)]
    blocks: [u8; 60],
}

impl Inode {
    pub fn is_dir(&self) -> bool {
        self.mode & 0xF000 == 0x4000
    }
    pub fn is_symlink(&self) -> bool {
        self.mode & 0xF000 == 0xA000
    }
    /// True if the inode still points at data blocks.
    pub fn has_blocks(&self) -> bool {
        self.blocks.iter().any(|&b| b != 0)
    }
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct DirEntry {
    pub name: String,
    pub inode: u32,
    pub file_type: u8,
    /// Recovered from directory slack (the file was deleted).
    pub deleted: bool,
}

impl<'a> ExtFs<'a> {
    pub fn open(src: &'a dyn ImageSource, base: u64) -> io::Result<Self> {
        let sb = src.read_vec(base + 1024, 1024)?;
        let bad = |m: &str| io::Error::new(io::ErrorKind::InvalidData, format!("ext at {base}: {m}"));
        if u16le(&sb, 56) != MAGIC {
            return Err(bad("no ext superblock"));
        }
        let log = u32le(&sb, 24);
        if log > 6 {
            return Err(bad("bad block size"));
        }
        let block_size = 1024u64 << log;
        let inodes_count = u32le(&sb, 0);
        let blocks_count = u32le(&sb, 4) as u64;
        let first_data_block = u32le(&sb, 20) as u64;
        let blocks_per_group = u32le(&sb, 32) as u64;
        let inodes_per_group = u32le(&sb, 40);
        let rev = u32le(&sb, 76);
        let inode_size = if rev >= 1 { u16le(&sb, 88) as u64 } else { 128 };
        let incompat = u32le(&sb, 96);
        if blocks_per_group == 0 || inodes_per_group == 0 || inode_size < 128 {
            return Err(bad("bad geometry"));
        }
        let desc_size = if incompat & INCOMPAT_64BIT != 0 { (u16le(&sb, 254) as u64).max(32) } else { 32 };
        let groups = (blocks_count - first_data_block).div_ceil(blocks_per_group);
        let gdt = src.read_vec(base + (first_data_block + 1) * block_size, (groups * desc_size) as usize)?;
        let inode_tables = (0..groups as usize)
            .map(|g| {
                let d = &gdt[g * desc_size as usize..];
                let lo = u32le(d, 8) as u64;
                let hi = if desc_size >= 64 { u32le(d, 0x28) as u64 } else { 0 };
                (hi << 32) | lo
            })
            .collect();
        let cstr = |b: &[u8]| String::from_utf8_lossy(b.split(|&c| c == 0).next().unwrap_or(&[])).into_owned();
        Ok(Self {
            src,
            base,
            block_size,
            inodes_count,
            inodes_per_group,
            inode_size,
            volume_name: cstr(&sb[120..136]),
            last_mounted: cstr(&sb[136..200]),
            uuid: sb[104..120].iter().map(|b| format!("{b:02x}")).collect(),
            created: ts(u32le(&sb, 264)),
            last_mount: ts(u32le(&sb, 44)),
            last_write: ts(u32le(&sb, 48)),
            filetype: incompat & INCOMPAT_FILETYPE != 0,
            inode_tables,
        })
    }

    pub fn inode(&self, n: u32) -> io::Result<Inode> {
        if n == 0 || n > self.inodes_count {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, format!("inode {n} out of range")));
        }
        let g = ((n - 1) / self.inodes_per_group) as usize;
        let idx = ((n - 1) % self.inodes_per_group) as u64;
        let table = *self.inode_tables.get(g).ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "group out of range"))?;
        let raw = self.src.read_vec(self.base + table * self.block_size + idx * self.inode_size, 128)?;
        let mode = u16le(&raw, 0);
        let size_hi = if mode & 0xF000 == 0x8000 { u32le(&raw, 108) as u64 } else { 0 };
        let mut blocks = [0u8; 60];
        blocks.copy_from_slice(&raw[40..100]);
        Ok(Inode {
            number: n,
            mode,
            size: (size_hi << 32) | u32le(&raw, 4) as u64,
            links: u16le(&raw, 26),
            flags: u32le(&raw, 32),
            atime: ts(u32le(&raw, 8)),
            ctime: ts(u32le(&raw, 12)),
            mtime: ts(u32le(&raw, 16)),
            dtime: ts(u32le(&raw, 20)),
            blocks,
        })
    }

    /// Disk extents of a file, in file order. Holes are skipped.
    pub fn extents(&self, ino: &Inode) -> io::Result<Vec<Extent>> {
        if ino.size == 0 || ino.is_symlink() && ino.size < 60 {
            return Ok(Vec::new());
        }
        let nblocks = ino.size.div_ceil(self.block_size);
        let mut runs: Vec<(u64, u64, u64)> = Vec::new(); // (logical, physical, count)
        if ino.flags & EXTENTS_FL != 0 {
            self.walk_extents(&ino.blocks, &mut runs, 0)?;
        } else {
            let p: Vec<u64> = ino.blocks.chunks_exact(4).map(|c| u32le(c, 0) as u64).collect();
            let mut logical = 0u64;
            for &b in &p[..12] {
                push_run(&mut runs, logical, b, 1);
                logical += 1;
            }
            for (level, &b) in p[12..15].iter().enumerate() {
                if logical >= nblocks {
                    break;
                }
                self.walk_indirect(b, level as u32 + 1, &mut logical, nblocks, &mut runs)?;
            }
        }
        let mut out = Vec::new();
        for (l, p, n) in runs {
            if p == 0 || l >= nblocks {
                continue;
            }
            let file_offset = l * self.block_size;
            let len = (n.min(nblocks - l) * self.block_size).min(ino.size - file_offset);
            out.push(Extent { file_offset, disk_offset: self.base + p * self.block_size, len });
        }
        Ok(out)
    }

    fn walk_indirect(&self, block: u64, level: u32, logical: &mut u64, nblocks: u64, runs: &mut Vec<(u64, u64, u64)>) -> io::Result<()> {
        let per = self.block_size / 4;
        let span = per.pow(level - 1);
        if block == 0 {
            *logical += per * span;
            return Ok(());
        }
        let data = self.src.read_vec(self.base + block * self.block_size, self.block_size as usize)?;
        for c in data.chunks_exact(4) {
            if *logical >= nblocks {
                break;
            }
            let b = u32le(c, 0) as u64;
            if level == 1 {
                push_run(runs, *logical, b, 1);
                *logical += 1;
            } else {
                self.walk_indirect(b, level - 1, logical, nblocks, runs)?;
            }
        }
        Ok(())
    }

    fn walk_extents(&self, node: &[u8], runs: &mut Vec<(u64, u64, u64)>, depth_guard: u32) -> io::Result<()> {
        if depth_guard > 5 || u16le(node, 0) != EXT_MAGIC {
            return Ok(());
        }
        let entries = u16le(node, 2) as usize;
        let depth = u16le(node, 6);
        for i in 0..entries {
            let e = &node[12 + i * 12..];
            if e.len() < 12 {
                break;
            }
            if depth == 0 {
                let len = u16le(e, 4) as u64;
                let len = if len > 32768 { len - 32768 } else { len }; // uninitialized extent
                let start = ((u16le(e, 6) as u64) << 32) | u32le(e, 8) as u64;
                runs.push((u32le(e, 0) as u64, start, len));
            } else {
                let leaf = ((u16le(e, 8) as u64) << 32) | u32le(e, 4) as u64;
                let child = self.src.read_vec(self.base + leaf * self.block_size, self.block_size as usize)?;
                self.walk_extents(&child, runs, depth_guard + 1)?;
            }
        }
        Ok(())
    }

    /// Read file bytes at `offset` (holes read as zeros).
    pub fn read(&self, ino: &Inode, offset: u64, len: usize) -> io::Result<Vec<u8>> {
        let end = (offset + len as u64).min(ino.size);
        if offset >= end {
            return Ok(Vec::new());
        }
        let mut out = vec![0u8; (end - offset) as usize];
        for x in self.extents(ino)? {
            let a = offset.max(x.file_offset);
            let b = end.min(x.file_offset + x.len);
            if a < b {
                let dst = &mut out[(a - offset) as usize..(b - offset) as usize];
                self.src.read_exact_at(x.disk_offset + (a - x.file_offset), dst)?;
            }
        }
        Ok(out)
    }

    /// Directory entries, live and (from slack space) deleted.
    pub fn list(&self, dir: &Inode) -> io::Result<Vec<DirEntry>> {
        if !dir.is_dir() {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "not a directory"));
        }
        let data = self.read(dir, 0, dir.size.min(64 << 20) as usize)?;
        let bs = self.block_size as usize;
        let mut out = Vec::new();
        for block in data.chunks(bs) {
            let mut pos = 0usize;
            while pos + 8 <= block.len() {
                let inode = u32le(block, pos);
                let rec_len = u16le(block, pos + 4) as usize;
                if rec_len < 8 || pos + rec_len > block.len() {
                    break;
                }
                let (name_len, ftype) = self.name_len_type(block, pos);
                let used = (8 + name_len).div_ceil(4) * 4;
                if name_len > 0 && 8 + name_len <= rec_len {
                    let name = String::from_utf8_lossy(&block[pos + 8..pos + 8 + name_len]).into_owned();
                    if name != "." && name != ".." {
                        // inode 0 in use position = first entry of a block that was deleted
                        out.push(DirEntry { name, inode, file_type: ftype, deleted: inode == 0 });
                    }
                }
                // Deleted entries hide in the slack after the real entry.
                let mut s = pos + used.max(8);
                while s + 8 <= pos + rec_len {
                    match self.slack_entry(block, s, pos + rec_len) {
                        Some((e, size)) => {
                            out.push(e);
                            s += size;
                        }
                        None => s += 4,
                    }
                }
                pos += rec_len;
            }
        }
        Ok(out)
    }

    fn name_len_type(&self, b: &[u8], pos: usize) -> (usize, u8) {
        if self.filetype {
            (b[pos + 6] as usize, b[pos + 7])
        } else {
            (u16le(b, pos + 6) as usize, 0)
        }
    }

    fn slack_entry(&self, b: &[u8], s: usize, limit: usize) -> Option<(DirEntry, usize)> {
        let inode = u32le(b, s);
        let rec_len = u16le(b, s + 4) as usize;
        let (name_len, ftype) = self.name_len_type(b, s);
        if inode == 0 || inode > self.inodes_count || name_len == 0 || s + 8 + name_len > limit {
            return None;
        }
        if rec_len < 8 + name_len || rec_len % 4 != 0 || (self.filetype && ftype > 7) {
            return None;
        }
        let raw = &b[s + 8..s + 8 + name_len];
        if raw.iter().any(|&c| c == 0 || c == b'/' || c < 0x20) {
            return None;
        }
        let name = std::str::from_utf8(raw).ok()?.to_string();
        if name == "." || name == ".." {
            return None;
        }
        Some((DirEntry { name, inode, file_type: ftype, deleted: true }, (8 + name_len).div_ceil(4) * 4))
    }
}

fn push_run(runs: &mut Vec<(u64, u64, u64)>, logical: u64, phys: u64, n: u64) {
    if let Some(last) = runs.last_mut() {
        if last.0 + last.2 == logical && phys != 0 && last.1 != 0 && last.1 + last.2 == phys {
            last.2 += n;
            return;
        }
    }
    runs.push((logical, phys, n));
}

fn ts(v: u32) -> Option<DateTime<Utc>> {
    (v != 0).then(|| DateTime::from_timestamp(v as i64, 0)).flatten()
}
fn u16le(b: &[u8], o: usize) -> u16 {
    u16::from_le_bytes([b[o], b[o + 1]])
}
fn u32le(b: &[u8], o: usize) -> u32 {
    u32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]])
}
