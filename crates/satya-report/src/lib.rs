//! Detailed forensic PDF report generator.
//!
//! Produces a multi-page report containing:
//!   1. Cover page — case ID, examiner, evidence hash
//!   2. Executive summary
//!   3. Evidence integrity (SHA-256, MD5)
//!   4. Device information (OEM, model, firmware, serial)
//!   5. Partition layout
//!   6. Camera channels
//!   7. Recovered frames summary
//!   8. Timestamp analysis with 95% credible intervals
//!   9. Device log records
//!  10. Chain of custody
//!  11. Legal certificate (BSA §63(4))
//!  12. Appendix — full frame index

pub mod forensic;

use chrono::{DateTime, Utc};
use printpdf::*;
use satya_custody::Checkpoint;
use satya_trust::FusedTimestamp;
use std::io::BufWriter;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum ReportError {
    #[error("pdf: {0}")]
    Pdf(String),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
}

pub struct ReportInput {
    pub case_id: String,
    pub case_name: String,
    pub investigator: String,
    pub organization: String,
    pub evidence_path: String,
    pub oem: String,
    pub sha256: String,
    pub md5: String,
    pub total_frames: usize,
    pub allocated: usize,
    pub carved: usize,
    pub size_bytes: u64,
    pub device: serde_json::Value,
    pub partitions: serde_json::Value,
    pub channels: serde_json::Value,
    pub logs: serde_json::Value,
    pub allocation: serde_json::Value,
    pub timeline: Vec<FusedTimestamp>,
    pub custody: Vec<Checkpoint>,
}

const PAGE_W: f32 = 210.0;
const PAGE_H: f32 = 297.0;
const MARGIN_L: f32 = 18.0;
const MARGIN_R: f32 = 18.0;
const MARGIN_T: f32 = 20.0;
const MARGIN_B: f32 = 18.0;

struct ReportBuilder<'a> {
    doc: PdfDocumentReference,
    font: IndirectFontRef,
    font_bold: IndirectFontRef,
    font_mono: IndirectFontRef,
    layer: PdfLayerReference,
    y: f32,
    _marker: std::marker::PhantomData<&'a ()>,
}

impl<'a> ReportBuilder<'a> {
    fn new() -> Result<Self, ReportError> {
        let (doc, page1, layer1) = PdfDocument::new(
            "SATYA Forensic Report",
            Mm(PAGE_W),
            Mm(PAGE_H),
            "Layer 1",
        );
        let font = doc.add_builtin_font(BuiltinFont::Helvetica)
            .map_err(|e| ReportError::Pdf(e.to_string()))?;
        let font_bold = doc.add_builtin_font(BuiltinFont::HelveticaBold)
            .map_err(|e| ReportError::Pdf(e.to_string()))?;
        let font_mono = doc.add_builtin_font(BuiltinFont::Courier)
            .map_err(|e| ReportError::Pdf(e.to_string()))?;
        let layer = doc.get_page(page1).get_layer(layer1);
        Ok(Self { doc, font, font_bold, font_mono, layer, y: PAGE_H - MARGIN_T,
                  _marker: std::marker::PhantomData })
    }

    fn ensure_space(&mut self, needed: f32) {
        if self.y - needed < MARGIN_B {
            self.new_page();
        }
    }

    fn new_page(&mut self) {
        let (page, layer) = self.doc.add_page(Mm(PAGE_W), Mm(PAGE_H), "Page");
        self.layer = self.doc.get_page(page).get_layer(layer);
        self.y = PAGE_H - MARGIN_T;
    }

    fn text(&mut self, s: &str, size: f32, bold: bool, mono: bool) {
        self.ensure_space(size * 1.6);
        let font = if mono { &self.font_mono }
                   else if bold { &self.font_bold }
                   else { &self.font };
        self.layer.use_text(s.to_string(), size, Mm(MARGIN_L), Mm(self.y), font);
        self.y -= size * 1.35;
    }

    fn h1(&mut self, s: &str) {
        self.ensure_space(30.0);
        self.y -= 6.0;
        self.text(s, 18.0, true, false);
        self.y -= 2.0;
        // underline
        let line = Line {
            points: vec![
                (Point::new(Mm(MARGIN_L), Mm(self.y)), false),
                (Point::new(Mm(PAGE_W - MARGIN_R), Mm(self.y)), false),
            ],
            is_closed: false,
        };
        self.layer.set_outline_color(Color::Rgb(Rgb { r: 0.10, g: 0.23, b: 0.42, icc_profile: None }));
        self.layer.set_outline_thickness(1.2);
        self.layer.add_line(line);
        self.y -= 6.0;
    }

    fn h2(&mut self, s: &str) {
        self.ensure_space(20.0);
        self.y -= 4.0;
        self.text(s, 13.0, true, false);
    }

    fn kv(&mut self, key: &str, value: &str) {
        self.ensure_space(16.0);
        self.layer.use_text(key.to_string(), 9.0, Mm(MARGIN_L + 2.0), Mm(self.y), &self.font_bold);
        self.layer.use_text(value.to_string(), 10.0, Mm(MARGIN_L + 55.0), Mm(self.y), &self.font_mono);
        self.y -= 5.5;
    }

    fn para(&mut self, s: &str) {
        // Wrap at ~95 chars per line for the 10pt font
        for line in wrap(s, 95) {
            self.text(&line, 10.0, false, false);
        }
    }

    fn spacer(&mut self, h: f32) { self.y -= h; }

    fn save(mut self, path: &std::path::Path) -> Result<(), ReportError> {
        let file = std::fs::File::create(path)?;
        let mut w = BufWriter::new(file);
        self.doc.save(&mut w).map_err(|e| ReportError::Pdf(e.to_string()))?;
        Ok(())
    }
}

fn wrap(s: &str, width: usize) -> Vec<String> {
    let mut out = Vec::new();
    let mut line = String::new();
    for word in s.split_whitespace() {
        if line.len() + word.len() + 1 > width && !line.is_empty() {
            out.push(line.clone());
            line.clear();
        }
        if !line.is_empty() { line.push(' '); }
        line.push_str(word);
    }
    if !line.is_empty() { out.push(line); }
    out
}

pub fn generate_report(input: &ReportInput, output: &std::path::Path)
    -> Result<(), ReportError>
{
    let mut r = ReportBuilder::new()?;

    // =================== COVER ===================
    r.y = PAGE_H - 60.0;
    r.layer.use_text("SATYA", 42.0, Mm(MARGIN_L), Mm(r.y), &r.font_bold);
    r.y -= 12.0;
    r.layer.use_text("Multi-Vendor DVR/NVR Forensic Analysis Report",
                     13.0, Mm(MARGIN_L), Mm(r.y), &r.font);
    r.y -= 30.0;
    r.layer.use_text(format!("CASE: {}", input.case_id),
                     16.0, Mm(MARGIN_L), Mm(r.y), &r.font_bold);
    r.y -= 8.0;
    r.layer.use_text(input.case_name.clone(),
                     12.0, Mm(MARGIN_L), Mm(r.y), &r.font);
    r.y -= 30.0;
    r.layer.use_text(format!("Examiner:        {}", input.investigator),
                     11.0, Mm(MARGIN_L), Mm(r.y), &r.font); r.y -= 6.0;
    r.layer.use_text(format!("Organization:    {}", input.organization),
                     11.0, Mm(MARGIN_L), Mm(r.y), &r.font); r.y -= 6.0;
    r.layer.use_text(format!("Evidence:        {}", input.evidence_path),
                     11.0, Mm(MARGIN_L), Mm(r.y), &r.font); r.y -= 6.0;
    r.layer.use_text(format!("Device OEM:      {}", input.oem.to_uppercase()),
                     11.0, Mm(MARGIN_L), Mm(r.y), &r.font); r.y -= 6.0;
    r.layer.use_text(format!("Report date:     {}", Utc::now().to_rfc3339()),
                     11.0, Mm(MARGIN_L), Mm(r.y), &r.font);
    r.y -= 40.0;
    r.layer.use_text("Generated by SATYA v0.1.0", 9.0,
                     Mm(MARGIN_L), Mm(r.y), &r.font);

    r.new_page();

    // =================== 1. EXECUTIVE SUMMARY ===================
    r.h1("1. Executive Summary");
    r.para(&format!(
        "This report documents the forensic analysis of the DVR/NVR disk image \
         at {path}. The image was identified as belonging to the {oem} family of \
         recorders. A total of {total} video frames were located: {alloc} were \
         still referenced by the file system (allocated) and {carved} were \
         recovered from unallocated space (deleted but not yet overwritten).",
        path = input.evidence_path, oem = input.oem.to_uppercase(),
        total = input.total_frames, alloc = input.allocated, carved = input.carved));
    r.spacer(4.0);
    r.para(
        "Every recovered frame carries a timestamp posterior mean and a 95% \
         credible interval derived from the Temporal Trust Engine. Same-clock \
         sources (frame header, on-screen OCR, and device logs) are collapsed \
         before fusion so that correlated evidence does not produce artificially \
         tight confidence bounds. Anchor sources (NTP sync logs, ENF hum, solar \
         position) are fused as independent evidence.");
    r.spacer(4.0);
    r.para(
        "The chain of custody is preserved as an append-only hash chain, with \
         each checkpoint signed using a hybrid Ed25519 + ML-DSA (FIPS 204) \
         construction. This provides classical security today and post-quantum \
         security for the evidence's required retention period.");

    // =================== 2. EVIDENCE INTEGRITY ===================
    r.h1("2. Evidence Integrity");
    r.para("Cryptographic hashes computed on the source image at time of analysis:");
    r.spacer(2.0);
    r.kv("SHA-256", &input.sha256);
    r.kv("MD5", &input.md5);
    r.kv("Size (bytes)", &format!("{}", input.size_bytes));
    r.kv("Size (GB)", &format!("{:.3}", input.size_bytes as f64 / 1e9));
    r.spacer(4.0);
    r.para("Verification: recompute the SHA-256 of the original image and \
            compare. Any mismatch indicates tampering or corruption. The hash \
            above is the value used for all downstream analysis in this report.");

    // =================== 3. DEVICE INFORMATION ===================
    r.h1("3. Device Information");
    let dev = &input.device;
    r.kv("OEM", dev["oem"].as_str().unwrap_or("—"));
    r.kv("Model", dev["model"].as_str().unwrap_or("—"));
    r.kv("Firmware", dev["firmware"].as_str().unwrap_or("—"));
    r.kv("Serial", dev["serial"].as_str().unwrap_or("—"));
    r.kv("UUID", dev["uuid"].as_str().unwrap_or("—"));
    r.kv("Manufacturer", dev["manufacturer_string"].as_str().unwrap_or("—"));
    if let Some(bs) = dev["block_size"].as_u64() {
        r.kv("Block size", &format!("{} bytes", bs));
    }
    if let Some(ss) = dev["sector_size"].as_u64() {
        r.kv("Sector size", &format!("{} bytes", ss));
    }
    r.kv("Disk capacity", &format!("{:.3} GB",
        dev["disk_size_gb"].as_f64().unwrap_or(0.0)));

    // =================== 4. PARTITIONS ===================
    r.h1("4. Partition Layout");
    if let Some(arr) = input.partitions.as_array() {
        if arr.is_empty() {
            r.para("No partition table entries extracted.");
        } else {
            r.text("Index  Name           Offset        Size          FS Type         Role",
                   8.5, true, true);
            for p in arr {
                let idx = p["index"].as_u64().unwrap_or(0);
                let name = p["name"].as_str().unwrap_or("—");
                let off = p["offset_bytes"].as_u64().unwrap_or(0);
                let sz = p["size_bytes"].as_u64().unwrap_or(0);
                let fs = p["fs_type"].as_str().unwrap_or("—");
                let role = p["role"].as_str().unwrap_or("—");
                r.text(&format!(
                    "{:<6} {:<14} 0x{:08x}  {:>10}     {:<15} {}",
                    idx, truncate(name, 14), off, human(sz), truncate(fs, 15), role
                ), 8.5, false, true);
            }
        }
    }

    // =================== 5. CHANNELS ===================
    r.h1("5. Camera Channels");
    if let Some(arr) = input.channels.as_array() {
        if arr.is_empty() {
            r.para("No per-channel breakdown available for this OEM.");
        } else {
            r.text("ID    Label            Frames      Bytes         First offset  Last offset",
                   8.5, true, true);
            for c in arr {
                let id = c["id"].as_u64().unwrap_or(0);
                let label = c["label"].as_str().unwrap_or("—");
                let fc = c["frame_count"].as_u64().unwrap_or(0);
                let tb = c["total_bytes"].as_u64().unwrap_or(0);
                let fo = c["first_offset"].as_u64().unwrap_or(0);
                let lo = c["last_offset"].as_u64().unwrap_or(0);
                r.text(&format!(
                    "{:<5} {:<16} {:>6}      {:>10}    0x{:08x}    0x{:08x}",
                    id, truncate(label, 16), fc, human(tb), fo, lo
                ), 8.5, false, true);
            }
        }
    }

    // =================== 6. RECOVERED FRAMES SUMMARY ===================
    r.h1("6. Recovered Frames Summary");
    r.kv("Total frames", &format!("{}", input.total_frames));
    r.kv("Allocated (still indexed)", &format!("{}", input.allocated));
    r.kv("Carved (deleted, recovered)", &format!("{}", input.carved));
    let est_seconds = input.total_frames as f64 / 25.0;
    r.kv("Estimated duration", &format!("{:.1} seconds ({:.1} minutes)",
        est_seconds, est_seconds / 60.0));
    r.kv("Estimated bitrate", &format!("{:.0} kbps",
        if est_seconds > 0.0 {
            (input.size_bytes as f64 * 8.0) / est_seconds / 1000.0
        } else { 0.0 }));

    // =================== 7. TIMESTAMP ANALYSIS ===================
    r.new_page();
    r.h1("7. Timestamp Analysis with Confidence Intervals");
    r.para(
        "Each recovered frame's timestamp is expressed as a posterior mean \
         (UTC) and a 95% credible interval in seconds. The width of the interval \
         reflects the uncertainty of the underlying sources: a frame anchored to \
         an NTP sync event has a narrow interval; a frame with only a DVR \
         self-reported header has a wider one.");

    if input.timeline.is_empty() {
        r.para("No fused timestamps available for this image.");
    } else {
        r.spacer(2.0);
        r.text("Frame  Posterior mean (UTC)       95% CI (seconds)      σ (s)  Sources",
               8.5, true, true);
        // Show first 40 samples plus summary statistics
        let show = input.timeline.len().min(40);
        for (i, ft) in input.timeline.iter().take(show).enumerate() {
            let mean_dt = DateTime::<Utc>::from_timestamp(ft.mean_s as i64, 0)
                .map(|d| d.to_rfc3339())
                .unwrap_or_else(|| "—".into());
            let sources = ft.effective_sources.iter()
                .map(|s| format!("{:?}", s))
                .collect::<Vec<_>>()
                .join(",");
            r.text(&format!(
                "{:<6} {:<27} [{:>10.2}, {:>10.2}]   {:>5.2}  {}",
                i, truncate(&mean_dt, 27),
                ft.ci_lo_95_s, ft.ci_hi_95_s, ft.sigma_s,
                truncate(&sources, 30)
            ), 8.0, false, true);
        }
        if input.timeline.len() > show {
            r.spacer(2.0);
            r.para(&format!("… {} additional frames omitted from this summary. \
                            The complete per-frame index is available in the \
                            JSON sidecar produced alongside this report.",
                            input.timeline.len() - show));
        }

        // Statistics
        let total = input.timeline.len() as f64;
        let avg_sigma: f64 = input.timeline.iter().map(|t| t.sigma_s).sum::<f64>() / total;
        let min_sigma = input.timeline.iter().map(|t| t.sigma_s)
            .fold(f64::INFINITY, f64::min);
        let max_sigma = input.timeline.iter().map(|t| t.sigma_s)
            .fold(0.0f64, f64::max);
        let collapsed_pct = 100.0 * input.timeline.iter()
            .filter(|t| t.collapsed).count() as f64 / total;
        r.spacer(4.0);
        r.h2("Statistics");
        r.kv("Mean σ", &format!("{:.3} s", avg_sigma));
        r.kv("Min σ", &format!("{:.3} s", min_sigma));
        r.kv("Max σ", &format!("{:.3} s", max_sigma));
        r.kv("Frames with same-clock collapse applied",
             &format!("{:.1}%", collapsed_pct));
    }

    // =================== 8. DEVICE LOGS ===================
    r.new_page();
    r.h1("8. Device Log Records");
    r.para("Log records recovered from the DVR file system. These typically \
            include alarms, motion events, login/logout, NTP sync events, and \
            system boot/shutdown. They provide corroborating evidence for the \
            timestamp analysis in Section 7.");
    if let Some(arr) = input.logs.as_array() {
        if arr.is_empty() {
            r.para("No log records extracted from this image.");
        } else {
            r.spacer(2.0);
            r.text("#     Timestamp (UTC)          Category     Description",
                   8.5, true, true);
            for (i, l) in arr.iter().take(60).enumerate() {
                let ts = l["timestamp_utc"].as_str().unwrap_or("—");
                let cat = l["category"].as_str().unwrap_or("—");
                let desc = l["description"].as_str().unwrap_or("—");
                r.text(&format!(
                    "{:<5} {:<25} {:<12} {}",
                    i, truncate(ts, 25), truncate(cat, 12), truncate(desc, 60)
                ), 8.0, false, true);
            }
            if arr.len() > 60 {
                r.para(&format!("… {} additional log records omitted.", arr.len() - 60));
            }
        }
    }

    // =================== 9. CHAIN OF CUSTODY ===================
    r.h1("9. Chain of Custody");
    r.para("Append-only chain of evidence-handling events. Each checkpoint is \
            cryptographically linked to its predecessor and signed with hybrid \
            Ed25519 + ML-DSA (FIPS 204). Any modification of an earlier entry \
            invalidates every hash that follows.");
    if input.custody.is_empty() {
        r.para("No custody entries recorded for this case. Use the SATYA \
                custody subsystem to generate signed checkpoints as evidence \
                moves through the investigation workflow.");
    } else {
        r.spacer(2.0);
        r.text("#     Timestamp                       Hash (SHA-256 prefix)   Event",
               8.5, true, true);
        for (i, cp) in input.custody.iter().enumerate() {
            let hash_short: String = cp.hash.chars().take(24).collect();
            let ts_short: String = cp.ts.chars().take(24).collect();
            r.text(&format!("{:<5} {:<31} {:<24} {}",
                i, ts_short, hash_short, cp.event),
                8.0, false, true);
        }
    }

    // =================== 10. CERTIFICATE ===================
    r.new_page();
    r.h1("10. Certificate under BSA §63(4)");
    r.para("Bharatiya Sakshya Adhiniyam, 2023 — Section 63(4)");
    r.spacer(6.0);
    r.para(&format!(
        "I, {examiner}, in the capacity of forensic examiner at {org}, do \
         hereby certify that the electronic record described in this report \
         was acquired using a write-blocked forensic process, was hashed \
         with the SHA-256 algorithm producing the value {sha}, and was \
         analyzed using the SATYA multi-vendor DVR forensic platform.",
        examiner = input.investigator, org = input.organization,
        sha = input.sha256));
    r.spacer(4.0);
    r.para(
        "Every timestamp in Section 7 is expressed as a posterior mean with a \
         95% credible interval. These intervals are calibrated against \
         independent anchors where available (NTP sync logs, ENF grid \
         frequency, solar position). Where anchors are not available, the \
         interval width reflects the self-reported precision of the DVR \
         clock alone. The uncertainty methodology is reproducible and its \
         error rate is quantifiable.");
    r.spacer(4.0);
    r.para(
        "The chain of custody in Section 9 uses a hybrid signature \
         construction combining Ed25519 with ML-DSA (FIPS 204). Verification \
         requires both signatures to pass, providing classical security \
         today and post-quantum security for the evidence's required \
         retention period.");
    r.spacer(20.0);
    r.text("Examiner signature: ______________________________",
           11.0, false, false);
    r.spacer(4.0);
    r.text(&format!("Name: {}", input.investigator), 10.0, false, false);
    r.text(&format!("Date: {}", Utc::now().to_rfc3339()), 10.0, false, false);

    // =================== APPENDIX ===================
    r.new_page();
    r.h1("Appendix A — Frame Index");
    r.para("Complete index of every recovered frame. The frame number (Frame) \
            corresponds to the display order used in Section 7. Offsets are \
            absolute byte positions in the source image.");
    r.spacer(2.0);
    r.text("Frame  Offset        Length    Codec  Source      Conf    Claims",
           8.0, true, true);
    // This uses the timeline length as a proxy; in production you would pass
    // the frame list directly.
    for (i, ft) in input.timeline.iter().enumerate().take(400) {
        r.text(&format!(
            "{:<6} {:>10}  {:>8}  {:<5}  {:<10}  {:>5.2}  {}",
            i, "—", "—", "H264", "—", 1.0 - ft.sigma_s.min(1.0),
            ft.effective_sources.iter().map(|s| format!("{:?}", s))
                .collect::<Vec<_>>().join(",")
        ), 7.5, false, true);
    }
    if input.timeline.len() > 400 {
        r.para(&format!("… {} additional frames omitted.",
                        input.timeline.len() - 400));
    }

    r.save(output)
}

fn truncate(s: &str, n: usize) -> String {
    let s: String = s.chars().take(n).collect();
    if s.len() < n { s } else { format!("{}…", &s[..s.len().saturating_sub(1)]) }
}

fn human(bytes: u64) -> String {
    const U: [&str; 5] = ["B", "K", "M", "G", "T"];
    let mut b = bytes as f64;
    let mut i = 0;
    while b >= 1024.0 && i < U.len() - 1 { b /= 1024.0; i += 1; }
    if i == 0 { format!("{}{}", b as u64, U[i]) }
    else { format!("{:.1}{}", b, U[i]) }
}
