//! Streaming evidence commands: image-info, verify, juan-scan, juan-export.
//! None of these load the image into memory or write to it.

use satya_image::{hashing, open_image, read_partitions, ImageSource};
use satya_parsers::juan::{ExportOptions, JuanScan, JuanVolume, ScanOptions, SlotState};
use serde::Serialize;
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};

type Res<T> = Result<T, Box<dyn std::error::Error>>;

const GIB: f64 = (1u64 << 30) as f64;

/// Legacy parsers take `&[u8]`; only allow that for images that fit comfortably.
pub fn legacy_bytes(src: &dyn ImageSource) -> Res<Vec<u8>> {
    const LIMIT: u64 = 4 << 30;
    if src.len() > LIMIT {
        return Err(format!(
            "no streaming parser matched, and legacy parsers need the image in RAM ({:.1} GiB > 4 GiB)",
            src.len() as f64 / GIB
        )
        .into());
    }
    Ok(src.read_vec(0, src.len() as usize)?)
}

fn progress_bar(label: &'static str) -> impl FnMut(u64, u64) {
    move |done, total| {
        let pct = if total == 0 { 100.0 } else { done as f64 * 100.0 / total as f64 };
        eprint!("\r{label}: {:.2} / {:.2} GiB ({pct:.0}%)   ", done as f64 / GIB, total as f64 / GIB);
        if done >= total {
            eprintln!();
        }
    }
}

pub fn cmd_image_info(image: &Path) -> Res<()> {
    let src = open_image(image)?;
    println!("Image:       {}", image.display());
    println!("Container:   {}", src.kind());
    println!("Media size:  {} bytes ({:.2} GiB, {} sectors)", src.len(), src.len() as f64 / GIB, src.len() / 512);
    let stored = src.stored_hashes();
    if let Some(m) = &stored.md5 {
        println!("Stored MD5:  {m}");
    }
    if let Some(s) = &stored.sha1 {
        println!("Stored SHA1: {s}");
    }
    println!("Partitions:");
    let parts = read_partitions(src.as_ref())?;
    if parts.is_empty() {
        println!("  (no partition table)");
    }
    for p in &parts {
        println!(
            "  #{} {} start {:>11} end {:>11}  {:>8.2} GiB  {:<7} {}",
            p.index,
            p.scheme,
            p.start_lba,
            p.end_lba,
            p.size() as f64 / GIB,
            p.fs.as_str(),
            p.name
        );
    }
    match JuanVolume::open(src.as_ref())? {
        Some(vol) => {
            println!("Device:      JUAN NVR family (confidence {:.2})", vol.layout.confidence);
            for s in &vol.layout.signals {
                println!("  - {s}");
            }
        }
        None => println!("Device:      no streaming parser matched"),
    }
    Ok(())
}

pub fn cmd_verify(image: &Path, output: Option<&PathBuf>) -> Res<()> {
    let src = open_image(image)?;
    eprintln!("Hashing {} of logical media; this reads the whole image.", fmt_gib(src.len()));
    let h = hashing::hash_media(src.as_ref(), progress_bar("hashed"))?;
    println!("Bytes:   {}", h.bytes);
    println!("MD5:     {}  {}", h.md5, verdict(h.md5_matches));
    println!("SHA-1:   {}  {}", h.sha1, verdict(h.sha1_matches));
    println!("SHA-256: {}", h.sha256);
    if let Some(out) = output {
        std::fs::write(out, serde_json::to_string_pretty(&h)?)?;
        println!("Wrote {}", out.display());
    }
    if h.md5_matches == Some(false) || h.sha1_matches == Some(false) {
        return Err("hash mismatch: the image does not match its acquisition hashes".into());
    }
    Ok(())
}

fn verdict(m: Option<bool>) -> &'static str {
    match m {
        Some(true) => "(matches stored)",
        Some(false) => "(DOES NOT MATCH stored)",
        None => "(no stored hash)",
    }
}

fn fmt_gib(b: u64) -> String {
    format!("{:.2} GiB", b as f64 / GIB)
}

fn open_juan(src: &dyn ImageSource) -> Res<JuanVolume<'_>> {
    JuanVolume::open(src)?.ok_or_else(|| "not a JUAN volume (no ident.bin/index.bin/dirNNNNN layout found)".into())
}

fn run_scan(vol: &JuanVolume, deep: bool, hash: bool) -> Res<JuanScan> {
    let mut last = usize::MAX;
    let scan = vol.scan(&ScanOptions { deep, hash_payloads: hash }, |done, total| {
        if done != last {
            eprint!("\rscanning slots: {done}/{total}   ");
            last = done;
            if done == total {
                eprintln!();
            }
        }
    })?;
    Ok(scan)
}

pub fn cmd_juan_scan(image: &Path, output: Option<&PathBuf>, deep: bool) -> Res<()> {
    let src = open_image(image)?;
    let vol = open_juan(src.as_ref())?;
    let scan = run_scan(&vol, deep, output.is_some())?;
    print_summary(&scan);
    if let Some(out) = output {
        let report = ScanReport { tool: tool_id(), image: image.display().to_string(), container: src.kind(), stored: src.stored_hashes(), scan: &scan };
        std::fs::write(out, serde_json::to_string_pretty(&report)?)?;
        println!("\nWrote slot-level report: {}", out.display());
    }
    Ok(())
}

#[derive(Serialize)]
struct ScanReport<'a> {
    tool: String,
    image: String,
    container: &'static str,
    stored: satya_image::StoredHashes,
    scan: &'a JuanScan,
}

fn print_summary(scan: &JuanScan) {
    let l = &scan.layout;
    let s = &scan.summary;
    println!("\n== Layout");
    println!(
        "FAT32 partition #{} at sector {}; {} slot dirs; {} slots{}; cluster {} bytes",
        l.fat_partition.index,
        l.fat_partition.start_lba,
        l.slot_dirs,
        l.slots,
        l.slot_size.map(|z| format!(" of {z} bytes")).unwrap_or_default(),
        l.cluster_size
    );
    if let Some(e) = &l.ext_partition {
        println!("System partition #{} ({}) at sector {}", e.index, e.fs.as_str(), e.start_lba);
    }
    println!("Deleted directory entries on FAT32: {}", l.deleted_dir_entries);

    println!("\n== Slots");
    println!(
        "recorded {}  empty {}  residual {}  unrecognized {}  (FAT-dated {})",
        s.recorded, s.empty, s.residual, s.unrecognized, s.fat_dated_slots
    );
    println!("\n== Timeline (header clock, printed as UTC)");
    println!("first start: {}", s.first_start_utc.map(|t| t.to_string()).unwrap_or("-".into()));
    println!("last end:    {}", s.last_end_utc.map(|t| t.to_string()).unwrap_or("-".into()));
    println!("recorded seconds (sum of slot spans): {}", s.recorded_seconds);
    println!("slot order monotonic: {}   overlaps: {}   gaps >5 s: {}", yn(s.slot_order_monotonic), s.overlaps, s.gaps.len());
    for (slot, gap) in s.gaps.iter().take(10) {
        println!("  gap before slot {slot}: {gap} s");
    }
    println!("FAT created minus header end (s) -> slots: {}", top(&s.fat_delta_hist, 6));
    println!("\n== Header / payload");
    println!("payload offset (header length) -> slots: {}", top(&s.header_len_hist, 6));
    let pct = if s.nal_total > 0 { s.nal_nonstandard as f64 * 100.0 / s.nal_total as f64 } else { 0.0 };
    println!("NAL units: {}  non-standard: {} ({pct:.2}%)", s.nal_total, s.nal_nonstandard);
    println!("pictures: {}  distinct SPS: {}  est. fps: {}", s.pictures, s.distinct_sps, s.est_fps.map(|f| format!("{f:.2}")).unwrap_or("-".into()));
    println!("payload bytes: {} ({})", s.payload_bytes, fmt_gib(s.payload_bytes));

    if let Some(r) = scan.slots.iter().find(|r| r.state == SlotState::Recorded) {
        println!("\n== First recorded slot: {}", r.path);
        println!("header start {} / end {}", r.header_start.unwrap_or(0), r.header_end.unwrap_or(0));
        println!("FAT created {:?}", r.fat_created);
        println!("NAL types (H.265 type -> count): {:?}", r.nal.types);
        if !r.nal.nonstandard_first_byte.is_empty() {
            println!("non-standard first bytes: {:?}", r.nal.nonstandard_first_byte);
        }
        if let Some(x) = r.payload_extents.first() {
            println!("payload starts at disk offset {} (sector {})", x.disk_offset, x.disk_offset / 512);
        }
    }

    println!("\n== Per directory (first 8)");
    for d in s.dirs.iter().filter(|d| d.recorded > 0).take(8) {
        println!(
            "dir{:05}: {}/{} recorded  {} -> {}",
            d.dir,
            d.recorded,
            d.slots,
            d.first_start_utc.map(|t| t.to_string()).unwrap_or("-".into()),
            d.last_end_utc.map(|t| t.to_string()).unwrap_or("-".into())
        );
    }

    println!("\n== Checks");
    if s.distinct_sps > 1 {
        println!("! {} distinct SPS: slots may interleave several cameras or stream profiles", s.distinct_sps);
    }
    if s.overlaps > 0 {
        println!("! {} slots overlap in time: slots may belong to different cameras", s.overlaps);
    }
    if !s.slot_order_monotonic {
        println!("! header times go backwards in slot order (ring-buffer wrap or per-camera dirs)");
    }
    if s.nal_nonstandard > 0 {
        println!("! non-standard NAL units present; consider --strip when exporting");
    }
    if s.residual > 0 {
        println!("! {} header-less slots still hold data: recovery candidates (juan-export --residual)", s.residual);
    }
    if s.recorded != s.fat_dated_slots {
        println!("! recorded slots ({}) != FAT-dated slots ({})", s.recorded, s.fat_dated_slots);
    }
}

fn yn(b: bool) -> &'static str {
    if b { "yes" } else { "NO" }
}

fn top<K: std::fmt::Display>(h: &std::collections::BTreeMap<K, usize>, n: usize) -> String {
    let mut v: Vec<_> = h.iter().collect();
    v.sort_by(|a, b| b.1.cmp(a.1));
    let shown: Vec<String> = v.iter().take(n).map(|(k, c)| format!("{k}:{c}")).collect();
    if shown.is_empty() { "-".into() } else { shown.join("  ") }
}

pub fn cmd_juan_export(image: &Path, output: &Path, strip: bool, from: Option<u32>, to: Option<u32>, residual: bool) -> Res<()> {
    let src = open_image(image)?;
    let vol = open_juan(src.as_ref())?;
    let scan = run_scan(&vol, residual, true)?;
    let range = match (from, to) {
        (None, None) => None,
        (a, b) => Some((a.unwrap_or(0), b.unwrap_or(u32::MAX))),
    };
    let opts = ExportOptions { strip_nonstandard: strip, slot_range: range, include_residual: residual };
    let file = std::fs::File::create(output)?;
    let mut w = BufWriter::new(file);
    let report = vol.export_hevc(&scan, &mut w, &opts)?;
    w.flush()?;

    let used: Vec<_> = scan.slots.iter().filter(|r| report.slot_order.contains(&r.slot)).collect();
    let manifest = Manifest {
        tool: tool_id(),
        created_utc: chrono::Utc::now().to_rfc3339(),
        image: image.display().to_string(),
        container: src.kind(),
        stored: src.stored_hashes(),
        output: output.display().to_string(),
        strip_nonstandard: strip,
        slot_range: range,
        include_residual: residual,
        export: &report,
        slots: used,
    };
    let manifest_path = output.with_extension("manifest.json");
    std::fs::write(&manifest_path, serde_json::to_string_pretty(&manifest)?)?;

    println!("Slots written: {}", report.slots_written);
    println!("Bytes:         {}", report.bytes_written);
    println!("NAL kept/dropped: {}/{}", report.nal_kept, report.nal_dropped);
    println!("MD5:           {}", report.md5);
    println!("SHA-256:       {}", report.sha256);
    println!("Manifest:      {}", manifest_path.display());
    let fps = report.est_fps.map(|f| format!("{:.2}", f)).unwrap_or_else(|| "25".into());
    println!("\nWrap without re-encoding (frame rate estimated from slot headers):");
    println!(
        "  ffmpeg -f hevc -framerate {fps} -i \"{}\" -c copy \"{}\"",
        output.display(),
        output.with_extension("mp4").display()
    );
    Ok(())
}

#[derive(Serialize)]
struct Manifest<'a> {
    tool: String,
    created_utc: String,
    image: String,
    container: &'static str,
    stored: satya_image::StoredHashes,
    output: String,
    strip_nonstandard: bool,
    slot_range: Option<(u32, u32)>,
    include_residual: bool,
    export: &'a satya_parsers::juan::ExportReport,
    slots: Vec<&'a satya_parsers::juan::SlotRecord>,
}

fn tool_id() -> String {
    format!("satya-cli {}", env!("CARGO_PKG_VERSION"))
}
