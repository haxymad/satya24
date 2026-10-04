//! GPT and MBR partition tables, plus a light filesystem probe.

use crate::source::ImageSource;
use std::io;

pub const SECTOR: u64 = 512;

#[derive(Debug, Clone, serde::Serialize)]
pub struct Partition {
    pub index: u32,
    pub scheme: &'static str,
    pub start_lba: u64,
    pub end_lba: u64,
    pub type_id: String,
    pub name: String,
    pub fs: FsKind,
}

impl Partition {
    pub fn offset(&self) -> u64 {
        self.start_lba * SECTOR
    }
    pub fn size(&self) -> u64 {
        (self.end_lba + 1 - self.start_lba) * SECTOR
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub enum FsKind {
    Fat32,
    Ext2,
    Ext3,
    Ext4,
    Ntfs,
    Unknown,
}

impl FsKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            FsKind::Fat32 => "FAT32",
            FsKind::Ext2 => "ext2",
            FsKind::Ext3 => "ext3",
            FsKind::Ext4 => "ext4",
            FsKind::Ntfs => "NTFS",
            FsKind::Unknown => "unknown",
        }
    }
}

/// Identify the filesystem that starts at `offset` from its superblock/boot sector.
pub fn probe_fs(src: &dyn ImageSource, offset: u64) -> FsKind {
    let mut boot = [0u8; 512];
    if src.read_exact_at(offset, &mut boot).is_ok() {
        if &boot[0x52..0x5A] == b"FAT32   " && boot[510] == 0x55 && boot[511] == 0xAA {
            return FsKind::Fat32;
        }
        if &boot[3..11] == b"NTFS    " {
            return FsKind::Ntfs;
        }
    }
    let mut sb = [0u8; 1024];
    if src.read_exact_at(offset + 1024, &mut sb).is_ok() && u16le(&sb, 0x38) == 0xEF53 {
        let compat = u32le(&sb, 0x5C);
        let incompat = u32le(&sb, 0x60);
        const COMPAT_HAS_JOURNAL: u32 = 0x4;
        const INCOMPAT_EXTENTS: u32 = 0x40;
        const INCOMPAT_64BIT: u32 = 0x80;
        const INCOMPAT_FLEX_BG: u32 = 0x200;
        if incompat & (INCOMPAT_EXTENTS | INCOMPAT_64BIT | INCOMPAT_FLEX_BG) != 0 {
            return FsKind::Ext4;
        }
        return if compat & COMPAT_HAS_JOURNAL != 0 { FsKind::Ext3 } else { FsKind::Ext2 };
    }
    FsKind::Unknown
}

/// Read the partition table: GPT if present, else MBR. An unpartitioned
/// volume yields an empty list.
pub fn read_partitions(src: &dyn ImageSource) -> io::Result<Vec<Partition>> {
    let mut hdr = [0u8; 512];
    src.read_exact_at(SECTOR, &mut hdr)?;
    if &hdr[0..8] == b"EFI PART" {
        return read_gpt(src, &hdr);
    }
    read_mbr(src)
}

fn read_gpt(src: &dyn ImageSource, hdr: &[u8]) -> io::Result<Vec<Partition>> {
    let entries_lba = u64le(hdr, 0x48);
    let count = u32le(hdr, 0x50).min(1024);
    let entry_size = u32le(hdr, 0x54) as usize;
    if entry_size < 128 || entry_size > 4096 {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "bad GPT entry size"));
    }
    let table = src.read_vec(entries_lba * SECTOR, count as usize * entry_size)?;
    let mut out = Vec::new();
    for i in 0..count as usize {
        let e = &table[i * entry_size..(i + 1) * entry_size];
        if e[0..16].iter().all(|&b| b == 0) {
            continue;
        }
        let start = u64le(e, 0x20);
        let end = u64le(e, 0x28);
        if end < start || end * SECTOR >= src.len() + SECTOR {
            continue;
        }
        let name_units: Vec<u16> = e[0x38..0x80]
            .chunks_exact(2)
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .take_while(|&u| u != 0)
            .collect();
        out.push(Partition {
            index: out.len() as u32,
            scheme: "gpt",
            start_lba: start,
            end_lba: end,
            type_id: guid_string(&e[0..16]),
            name: String::from_utf16_lossy(&name_units),
            fs: probe_fs(src, start * SECTOR),
        });
    }
    Ok(out)
}

fn read_mbr(src: &dyn ImageSource) -> io::Result<Vec<Partition>> {
    let mbr = src.read_vec(0, 512)?;
    if mbr[510] != 0x55 || mbr[511] != 0xAA {
        return Ok(Vec::new());
    }
    let mut out = Vec::new();
    for i in 0..4 {
        let e = &mbr[0x1BE + i * 16..0x1BE + (i + 1) * 16];
        let ptype = e[4];
        let start = u32le(e, 8) as u64;
        let sectors = u32le(e, 12) as u64;
        // 0x05/0x0F extended containers are not walked yet.
        if ptype == 0 || sectors == 0 || ptype == 0x05 || ptype == 0x0F {
            continue;
        }
        out.push(Partition {
            index: out.len() as u32,
            scheme: "mbr",
            start_lba: start,
            end_lba: start + sectors - 1,
            type_id: format!("0x{ptype:02X}"),
            name: String::new(),
            fs: probe_fs(src, start * SECTOR),
        });
    }
    Ok(out)
}

fn guid_string(b: &[u8]) -> String {
    format!(
        "{:08X}-{:04X}-{:04X}-{}-{}",
        u32le(b, 0),
        u16le(b, 4),
        u16le(b, 6),
        hex::encode_upper(&b[8..10]),
        hex::encode_upper(&b[10..16])
    )
}

pub(crate) fn u16le(b: &[u8], o: usize) -> u16 {
    u16::from_le_bytes([b[o], b[o + 1]])
}
pub(crate) fn u32le(b: &[u8], o: usize) -> u32 {
    u32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]])
}
pub(crate) fn u64le(b: &[u8], o: usize) -> u64 {
    u64::from_le_bytes(b[o..o + 8].try_into().unwrap())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::source::MemImage;

    #[test]
    fn parses_minimal_gpt() {
        let mut img = vec![0u8; 64 * 512];
        img[510] = 0x55;
        img[511] = 0xAA;
        let h = 512;
        img[h..h + 8].copy_from_slice(b"EFI PART");
        img[h + 0x48..h + 0x50].copy_from_slice(&2u64.to_le_bytes());
        img[h + 0x50..h + 0x54].copy_from_slice(&4u32.to_le_bytes());
        img[h + 0x54..h + 0x58].copy_from_slice(&128u32.to_le_bytes());
        let e = 2 * 512;
        img[e] = 0xAF; // non-zero type GUID
        img[e + 0x20..e + 0x28].copy_from_slice(&34u64.to_le_bytes());
        img[e + 0x28..e + 0x30].copy_from_slice(&40u64.to_le_bytes());
        let parts = read_partitions(&MemImage(&img)).unwrap();
        assert_eq!(parts.len(), 1);
        assert_eq!(parts[0].start_lba, 34);
        assert_eq!(parts[0].size(), 7 * 512);
        assert_eq!(parts[0].scheme, "gpt");
    }
}
