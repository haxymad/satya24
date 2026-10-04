//! Minimal read-only FAT32 reader.
//!
//! Written in-house rather than pulled from a crate so that every byte we
//! report can be traced to an absolute disk offset (cluster -> sector), and
//! so that deleted directory entries are surfaced instead of skipped.
//!
//! FAT timestamps are local time with no timezone, so they are returned as
//! `NaiveDateTime`; callers decide how to normalise them.

use crate::partition::{u16le, u32le};
use crate::source::ImageSource;
use chrono::{NaiveDate, NaiveDateTime};
use std::io;

const ATTR_DIR: u8 = 0x10;
const ATTR_VOLUME: u8 = 0x08;
const ATTR_LFN: u8 = 0x0F;
const EOC_MIN: u32 = 0x0FFF_FFF8;
const BAD_CLUSTER: u32 = 0x0FFF_FFF7;

pub struct Fat32<'a> {
    src: &'a dyn ImageSource,
    /// Absolute offset of the volume (partition start).
    pub base: u64,
    pub bytes_per_sector: u64,
    pub cluster_size: u64,
    pub cluster_count: u32,
    pub root_cluster: u32,
    /// Absolute offset of cluster #2.
    pub data_offset: u64,
    /// Absolute offset of the first FAT.
    pub fat_offset: u64,
    pub volume_id: u32,
    pub volume_label: String,
    fat: Vec<u32>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct FatEntry {
    pub name: String,
    pub short_name: String,
    pub is_dir: bool,
    pub deleted: bool,
    pub attr: u8,
    pub first_cluster: u32,
    pub size: u32,
    /// Local time as written by the device (no timezone).
    pub created: Option<NaiveDateTime>,
    pub modified: Option<NaiveDateTime>,
    pub accessed: Option<NaiveDate>,
    /// Absolute offset of the 32-byte short directory entry.
    pub dirent_offset: u64,
}

/// One contiguous run of a file on disk.
#[derive(Debug, Clone, Copy, serde::Serialize)]
pub struct Extent {
    pub file_offset: u64,
    pub disk_offset: u64,
    pub len: u64,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct FileMap {
    pub extents: Vec<Extent>,
    pub size: u64,
    /// False when the cluster chain ended before `size` bytes were mapped,
    /// or when the layout was assumed (deleted entry, chain unavailable).
    pub complete: bool,
}

impl FileMap {
    pub fn is_contiguous(&self) -> bool {
        self.extents.len() <= 1
    }
}

impl<'a> Fat32<'a> {
    /// Open the FAT32 volume starting at absolute byte offset `base`.
    pub fn open(src: &'a dyn ImageSource, base: u64) -> io::Result<Self> {
        let boot = src.read_vec(base, 512)?;
        let bad = |m: &str| io::Error::new(io::ErrorKind::InvalidData, format!("FAT32 at {base}: {m}"));
        if &boot[0x52..0x5A] != b"FAT32   " {
            return Err(bad("missing FAT32 signature"));
        }
        let bps = u16le(&boot, 0x0B) as u64;
        let spc = boot[0x0D] as u64;
        let reserved = u16le(&boot, 0x0E) as u64;
        let nfats = boot[0x10] as u64;
        let total16 = u16le(&boot, 0x13) as u64;
        let total32 = u32le(&boot, 0x20) as u64;
        let fat_sectors = u32le(&boot, 0x24) as u64;
        if ![512, 1024, 2048, 4096].contains(&bps) || spc == 0 || !spc.is_power_of_two() {
            return Err(bad("bad geometry"));
        }
        if nfats == 0 || nfats > 2 || fat_sectors == 0 {
            return Err(bad("bad FAT layout"));
        }
        let total = if total16 != 0 { total16 } else { total32 };
        let data_start = reserved + nfats * fat_sectors;
        if total <= data_start {
            return Err(bad("volume smaller than its metadata"));
        }
        let cluster_count = ((total - data_start) / spc) as u32;
        let fat_offset = base + reserved * bps;
        let entries = (cluster_count as u64 + 2).min(fat_sectors * bps / 4);
        if fat_offset + entries * 4 > src.len() {
            return Err(bad("FAT extends past end of media"));
        }
        let entries = entries as usize;

        // Load FAT #1 in 1 MiB reads (35 MB for a 150 GB volume).
        let mut fat = Vec::with_capacity(entries);
        let mut buf = vec![0u8; 1 << 20];
        let mut done = 0usize;
        while done < entries {
            let n = (entries - done).min(buf.len() / 4);
            src.read_exact_at(fat_offset + done as u64 * 4, &mut buf[..n * 4])?;
            fat.extend(buf[..n * 4].chunks_exact(4).map(|c| u32le(c, 0) & 0x0FFF_FFFF));
            done += n;
        }

        let label = String::from_utf8_lossy(&boot[0x47..0x52]).trim().to_string();
        Ok(Self {
            src,
            base,
            bytes_per_sector: bps,
            cluster_size: bps * spc,
            cluster_count,
            root_cluster: u32le(&boot, 0x2C),
            data_offset: base + data_start * bps,
            fat_offset,
            volume_id: u32le(&boot, 0x43),
            volume_label: label,
            fat,
        })
    }

    fn valid_cluster(&self, c: u32) -> bool {
        c >= 2 && (c as u64) < self.cluster_count as u64 + 2 && (c as usize) < self.fat.len()
    }

    /// Absolute disk offset of a data cluster.
    pub fn cluster_offset(&self, c: u32) -> u64 {
        self.data_offset + (c as u64 - 2) * self.cluster_size
    }

    /// Follow the FAT from `first`, up to `max` clusters. Stops on end-of-chain,
    /// bad/free/out-of-range links, or a loop.
    pub fn chain(&self, first: u32, max: usize) -> Vec<u32> {
        let mut out = Vec::new();
        let mut c = first;
        while self.valid_cluster(c) && out.len() < max && out.len() <= self.cluster_count as usize {
            out.push(c);
            let next = self.fat[c as usize];
            if next >= EOC_MIN || next == BAD_CLUSTER || next < 2 {
                break;
            }
            c = next;
        }
        out
    }

    /// Map a file's bytes to disk extents.
    pub fn file_map(&self, e: &FatEntry) -> FileMap {
        let size = e.size as u64;
        if size == 0 || !self.valid_cluster(e.first_cluster) {
            return FileMap { extents: Vec::new(), size, complete: size == 0 };
        }
        let need = size.div_ceil(self.cluster_size) as usize;
        let (clusters, assumed) = if e.deleted {
            // FAT links of deleted files are zeroed: assume contiguous (standard practice).
            ((0..need as u32).map(|i| e.first_cluster + i).collect::<Vec<_>>(), true)
        } else {
            (self.chain(e.first_cluster, need), false)
        };
        let mut extents: Vec<Extent> = Vec::new();
        for (i, &c) in clusters.iter().enumerate() {
            let file_offset = i as u64 * self.cluster_size;
            let len = self.cluster_size.min(size - file_offset);
            let disk_offset = self.cluster_offset(c);
            match extents.last_mut() {
                Some(last) if last.disk_offset + last.len == disk_offset => last.len += len,
                _ => extents.push(Extent { file_offset, disk_offset, len }),
            }
        }
        let mapped: u64 = extents.iter().map(|x| x.len).sum();
        FileMap { extents, size, complete: !assumed && mapped == size }
    }

    /// Read file bytes at `offset` through its map.
    pub fn read_at(&self, map: &FileMap, offset: u64, buf: &mut [u8]) -> io::Result<usize> {
        let mut done = 0usize;
        while done < buf.len() {
            let pos = offset + done as u64;
            let Some(x) = map.extents.iter().find(|x| pos >= x.file_offset && pos < x.file_offset + x.len) else {
                break;
            };
            let within = pos - x.file_offset;
            let n = ((x.len - within) as usize).min(buf.len() - done);
            self.src.read_exact_at(x.disk_offset + within, &mut buf[done..done + n])?;
            done += n;
        }
        Ok(done)
    }

    pub fn read_file(&self, e: &FatEntry) -> io::Result<Vec<u8>> {
        let map = self.file_map(e);
        let mapped: u64 = map.extents.iter().map(|x| x.len).sum();
        let mut v = vec![0u8; mapped as usize];
        let n = self.read_at(&map, 0, &mut v)?;
        v.truncate(n);
        Ok(v)
    }

    pub fn list_root(&self) -> io::Result<Vec<FatEntry>> {
        self.list_cluster(self.root_cluster)
    }

    pub fn list_dir(&self, dir: &FatEntry) -> io::Result<Vec<FatEntry>> {
        if !dir.is_dir {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, format!("{} is not a directory", dir.name)));
        }
        self.list_cluster(dir.first_cluster)
    }

    /// Resolve a path like "/dir00000/file0000.dat" (case-insensitive, live entries only).
    pub fn find(&self, path: &str) -> io::Result<Option<FatEntry>> {
        let mut entries = self.list_root()?;
        let mut found: Option<FatEntry> = None;
        let parts: Vec<&str> = path.split('/').filter(|p| !p.is_empty()).collect();
        for (i, part) in parts.iter().enumerate() {
            let Some(hit) = entries.iter().find(|e| !e.deleted && e.name.eq_ignore_ascii_case(part)).cloned() else {
                return Ok(None);
            };
            if i + 1 < parts.len() {
                entries = self.list_dir(&hit)?;
            }
            found = Some(hit);
        }
        Ok(found)
    }

    fn list_cluster(&self, first: u32) -> io::Result<Vec<FatEntry>> {
        // Directories are small; read the whole chain (cap at 64 MiB of entries).
        let max = ((64u64 << 20) / self.cluster_size).max(1) as usize;
        let clusters = self.chain(first, max);
        let mut out = Vec::new();
        let mut lfn: Vec<(u8, [u16; 13], u8)> = Vec::new();
        let mut buf = vec![0u8; self.cluster_size as usize];
        'outer: for c in clusters {
            let base = self.cluster_offset(c);
            self.src.read_exact_at(base, &mut buf)?;
            for (i, e) in buf.chunks_exact(32).enumerate() {
                if e[0] == 0x00 {
                    break 'outer;
                }
                let attr = e[11];
                let deleted = e[0] == 0xE5;
                if attr == ATTR_LFN {
                    if !deleted {
                        let mut units = [0u16; 13];
                        let pos = [1, 3, 5, 7, 9, 14, 16, 18, 20, 22, 24, 28, 30];
                        for (k, &p) in pos.iter().enumerate() {
                            units[k] = u16le(e, p);
                        }
                        lfn.push((e[0] & 0x1F, units, e[13]));
                    }
                    continue;
                }
                if attr & ATTR_VOLUME != 0 {
                    lfn.clear();
                    continue;
                }
                let short = short_name(e, deleted);
                if short == "." || short == ".." {
                    lfn.clear();
                    continue;
                }
                let name = long_name(&lfn, &e[0..11]).unwrap_or_else(|| short.clone());
                lfn.clear();
                let cluster = ((u16le(e, 0x14) as u32) << 16) | u16le(e, 0x1A) as u32;
                out.push(FatEntry {
                    name,
                    short_name: short,
                    is_dir: attr & ATTR_DIR != 0,
                    deleted,
                    attr,
                    first_cluster: cluster,
                    size: u32le(e, 0x1C),
                    created: dos_datetime(u16le(e, 0x10), u16le(e, 0x0E), e[0x0D]),
                    modified: dos_datetime(u16le(e, 0x18), u16le(e, 0x16), 0),
                    accessed: dos_date(u16le(e, 0x12)),
                    dirent_offset: base + i as u64 * 32,
                });
            }
        }
        Ok(out)
    }
}

fn short_name(e: &[u8], deleted: bool) -> String {
    let mut raw = e[0..11].to_vec();
    if deleted {
        raw[0] = b'_';
    } else if raw[0] == 0x05 {
        raw[0] = 0xE5;
    }
    let lower_base = e[12] & 0x08 != 0;
    let lower_ext = e[12] & 0x10 != 0;
    let conv = |s: &[u8], lower: bool| {
        let t = String::from_utf8_lossy(s).trim_end().to_string();
        if lower { t.to_ascii_lowercase() } else { t }
    };
    let base = conv(&raw[0..8], lower_base);
    let ext = conv(&raw[8..11], lower_ext);
    if ext.is_empty() { base } else { format!("{base}.{ext}") }
}

fn long_name(parts: &[(u8, [u16; 13], u8)], short_raw: &[u8]) -> Option<String> {
    if parts.is_empty() {
        return None;
    }
    let sum = short_raw.iter().fold(0u8, |s, &b| s.rotate_right(1).wrapping_add(b));
    if parts.iter().any(|p| p.2 != sum) {
        return None;
    }
    let mut ordered = parts.to_vec();
    ordered.sort_by_key(|p| p.0);
    let units: Vec<u16> = ordered
        .iter()
        .flat_map(|p| p.1.iter().copied())
        .take_while(|&u| u != 0x0000)
        .filter(|&u| u != 0xFFFF)
        .collect();
    Some(String::from_utf16_lossy(&units))
}

fn dos_date(d: u16) -> Option<NaiveDate> {
    if d == 0 {
        return None;
    }
    NaiveDate::from_ymd_opt(1980 + (d >> 9) as i32, ((d >> 5) & 0x0F) as u32, (d & 0x1F) as u32)
}

fn dos_datetime(d: u16, t: u16, centis: u8) -> Option<NaiveDateTime> {
    let date = dos_date(d)?;
    let centis = (centis as u32).min(199);
    let secs = (t & 0x1F) as u32 * 2 + centis / 100;
    date.and_hms_milli_opt((t >> 11) as u32, ((t >> 5) & 0x3F) as u32, secs, (centis % 100) * 10)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_dos_timestamps() {
        // 2021-08-04 06:01:32 + 1.00 s from the 10 ms field = 06:01:33
        let date = ((2021 - 1980) << 9) | (8 << 5) | 4;
        let time = (6 << 11) | (1 << 5) | (32 / 2);
        let dt = dos_datetime(date, time, 100).unwrap();
        assert_eq!(dt.to_string(), "2021-08-04 06:01:33");
        assert!(dos_datetime(0, 0, 0).is_none());
    }

    #[test]
    fn short_name_honours_nt_lowercase_flags() {
        let mut e = [0u8; 32];
        e[0..11].copy_from_slice(b"FILE0000DAT");
        e[12] = 0x08 | 0x10;
        assert_eq!(short_name(&e, false), "file0000.dat");
        assert_eq!(short_name(&e, true), "_ile0000.dat");
    }
}
