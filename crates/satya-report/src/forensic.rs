//! Detailed forensic examination report (PDF).
//!
//! Layout is done in millimetres throughout. Every page gets a header and a
//! "Page N of M" footer, and the table of contents carries real page numbers.
//! Text is limited to the PDF built-in fonts (WinAnsi), so `clean()` maps
//! common Unicode punctuation to ASCII and drops anything else.

use printpdf::image_crate::{self, DynamicImage};
use printpdf::path::PaintMode;
use printpdf::*;
use serde::Serialize;
use std::io::BufWriter;
use std::path::Path;

use crate::ReportError;

// ---------------------------------------------------------------------------
// Input
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default, Serialize)]
pub struct ForensicReport {
    pub case_id: String,
    pub case_name: String,
    pub examiner: String,
    pub organization: String,
    pub notes: String,
    pub generated_utc: String,
    pub tool: String,
    pub evidence: Evidence,
    pub device: DeviceView,
    pub disk_bytes: u64,
    pub partitions: Vec<PartitionRow>,
    pub recording: Recording,
    pub thumbnails: Vec<Thumbnail>,
    pub slots: Vec<SlotRow>,
    pub deleted: Vec<DeletedRow>,
    pub exports: Vec<ExportRow>,
    pub activity: Vec<ActivityRow>,
    pub session_public_key: String,
    pub findings: Vec<String>,
    pub warnings: Vec<String>,
    pub limitations: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct Evidence {
    pub path: String,
    pub container: String,
    pub media_bytes: u64,
    pub stored_md5: Option<String>,
    pub stored_sha1: Option<String>,
    /// Full re-hash done in SATYA this session (None = not run).
    pub verified: Option<Verification>,
    pub acquisition: Vec<(String, String)>,
    pub read_errors: usize,
    /// (file, bytes, sha256 of the container file)
    pub segments: Vec<(String, u64, String)>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct Verification {
    pub md5: String,
    pub sha1: String,
    pub sha256: String,
    pub md5_matches: Option<bool>,
    pub sha1_matches: Option<bool>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct DeviceView {
    pub family: String,
    pub model: String,
    pub manufacturer: String,
    pub confidence: f32,
    pub signals: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct PartitionRow {
    pub index: u32,
    pub fs: String,
    pub start_lba: u64,
    pub bytes: u64,
    pub role: String,
    pub detail: String,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct Recording {
    pub first_utc: String,
    pub last_utc: String,
    pub recorded_seconds: i64,
    pub slots_total: usize,
    pub recorded: usize,
    pub empty: usize,
    pub residual: usize,
    pub unrecognized: usize,
    pub slot_bytes: u64,
    pub payload_bytes: u64,
    pub monotonic: bool,
    pub overlaps: usize,
    pub gaps: Vec<(u32, i64)>,
    pub est_fps: Option<f64>,
    pub distinct_sps: usize,
    pub nal_total: u64,
    pub nal_nonstandard: u64,
    pub pictures: u64,
    pub header_len: Vec<(u64, usize)>,
    pub fat_delta: Vec<(i64, usize)>,
    /// (label, minutes recorded) per hour of the header clock.
    pub hourly: Vec<(String, f64)>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct Thumbnail {
    #[serde(skip)]
    pub jpeg: Vec<u8>,
    pub caption: String,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct SlotRow {
    pub slot: u32,
    pub path: String,
    pub start: String,
    pub end: String,
    pub seconds: i64,
    pub bytes: u64,
    pub sector: u64,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct DeletedRow {
    pub location: String,
    pub path: String,
    pub id: String,
    pub size: u64,
    pub deleted_at: String,
    pub status: String,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct ExportRow {
    pub file: String,
    pub bytes: u64,
    pub md5: String,
    pub sha256: String,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct ActivityRow {
    pub time: String,
    pub action: String,
    pub detail: String,
    pub hash: String,
}

// ---------------------------------------------------------------------------
// Page geometry and colours
// ---------------------------------------------------------------------------

const W: f32 = 210.0;
const H: f32 = 297.0;
const ML: f32 = 18.0;
const MR: f32 = 18.0;
const TOP: f32 = 22.0; // below the running header
const BOTTOM: f32 = 20.0; // above the footer
const CW: f32 = W - ML - MR;
const PT: f32 = 0.3528; // mm per point

fn rgb(r: f32, g: f32, b: f32) -> Color {
    Color::Rgb(Rgb::new(r, g, b, None))
}
fn navy() -> Color {
    rgb(0.10, 0.20, 0.36)
}
fn accent() -> Color {
    rgb(0.80, 0.20, 0.30)
}
fn grey(v: f32) -> Color {
    rgb(v, v, v)
}

/// Built-in PDF fonts only cover WinAnsi; keep text safe and readable.
pub fn clean(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '\u{2014}' | '\u{2013}' | '\u{2212}' => out.push('-'),
            '\u{2026}' => out.push_str("..."),
            '\u{2018}' | '\u{2019}' => out.push('\''),
            '\u{201C}' | '\u{201D}' => out.push('"'),
            '\u{2192}' => out.push_str("->"),
            '\u{2190}' => out.push_str("<-"),
            '\u{00B7}' | '\u{2022}' => out.push('-'),
            '\u{03C3}' => out.push_str("sigma"),
            '\u{00D7}' => out.push('x'),
            '\u{2264}' => out.push_str("<="),
            '\u{2265}' => out.push_str(">="),
            '\u{00A7}' => out.push_str("Sec."),
            c if (' '..='~').contains(&c) => out.push(c),
            '\t' => out.push(' '),
            _ => {}
        }
    }
    out
}

pub fn human_bytes(b: u64) -> String {
    const U: [&str; 6] = ["B", "KiB", "MiB", "GiB", "TiB", "PiB"];
    let mut v = b as f64;
    let mut i = 0;
    while v >= 1024.0 && i < U.len() - 1 {
        v /= 1024.0;
        i += 1;
    }
    if i == 0 { format!("{b} B") } else { format!("{v:.2} {}", U[i]) }
}

pub fn human_secs(s: i64) -> String {
    let (h, m, sec) = (s / 3600, (s % 3600) / 60, s % 60);
    if h > 0 { format!("{h} h {m:02} min {sec:02} s") } else if m > 0 { format!("{m} min {sec:02} s") } else { format!("{sec} s") }
}

// ---------------------------------------------------------------------------
// Layout engine
// ---------------------------------------------------------------------------

struct Doc {
    doc: PdfDocumentReference,
    font: IndirectFontRef,
    bold: IndirectFontRef,
    mono: IndirectFontRef,
    pages: Vec<(PdfPageIndex, PdfLayerIndex)>,
    layer: PdfLayerReference,
    y: f32,
    toc: Vec<(String, usize)>,
}

impl Doc {
    fn new(title: &str) -> Result<Self, ReportError> {
        let (doc, p, l) = PdfDocument::new(clean(title), Mm(W), Mm(H), "content");
        let font = doc.add_builtin_font(BuiltinFont::Helvetica).map_err(|e| ReportError::Pdf(e.to_string()))?;
        let bold = doc.add_builtin_font(BuiltinFont::HelveticaBold).map_err(|e| ReportError::Pdf(e.to_string()))?;
        let mono = doc.add_builtin_font(BuiltinFont::Courier).map_err(|e| ReportError::Pdf(e.to_string()))?;
        let layer = doc.get_page(p).get_layer(l);
        Ok(Self { doc, font, bold, mono, pages: vec![(p, l)], layer, y: H - TOP, toc: Vec::new() })
    }

    fn page(&mut self) {
        let (p, l) = self.doc.add_page(Mm(W), Mm(H), "content");
        self.pages.push((p, l));
        self.layer = self.doc.get_page(p).get_layer(l);
        self.y = H - TOP;
    }

    fn need(&mut self, mm: f32) {
        if self.y - mm < BOTTOM {
            self.page();
        }
    }

    fn text_at(&self, s: &str, pt: f32, x: f32, y: f32, f: &IndirectFontRef, c: Color) {
        self.layer.set_fill_color(c);
        self.layer.use_text(clean(s), pt, Mm(x), Mm(y), f);
    }

    fn rect(&self, x: f32, y: f32, w: f32, h: f32, c: Color) {
        self.layer.set_fill_color(c);
        self.layer.add_rect(Rect::new(Mm(x), Mm(y), Mm(x + w), Mm(y + h)).with_mode(PaintMode::Fill));
    }

    fn line(&self, x1: f32, y1: f32, x2: f32, y2: f32, c: Color, w: f32) {
        self.layer.set_outline_color(c);
        self.layer.set_outline_thickness(w);
        self.layer.add_line(Line {
            points: vec![(Point::new(Mm(x1), Mm(y1)), false), (Point::new(Mm(x2), Mm(y2)), false)],
            is_closed: false,
        });
    }

    /// Wrap to the content width (Helvetica average glyph ~0.5 em).
    fn wrap(s: &str, pt: f32, width: f32) -> Vec<String> {
        let max = ((width / (pt * PT * 0.5)) as usize).max(10);
        let mut out = Vec::new();
        for para in clean(s).split('\n') {
            let mut line = String::new();
            for word in para.split_whitespace() {
                if !line.is_empty() && line.len() + 1 + word.len() > max {
                    out.push(std::mem::take(&mut line));
                }
                if !line.is_empty() {
                    line.push(' ');
                }
                line.push_str(word);
            }
            out.push(line);
        }
        out
    }

    fn para(&mut self, s: &str) {
        self.para_sized(s, 10.0, grey(0.12));
    }

    fn para_sized(&mut self, s: &str, pt: f32, c: Color) {
        let lh = pt * PT * 1.45;
        for l in Self::wrap(s, pt, CW) {
            self.need(lh);
            self.text_at(&l, pt, ML, self.y - pt * PT, &self.font.clone(), c.clone());
            self.y -= lh;
        }
        self.y -= 1.5;
    }

    fn bullets(&mut self, items: &[String]) {
        let pt = 10.0;
        let lh = pt * PT * 1.45;
        for it in items {
            let lines = Self::wrap(it, pt, CW - 6.0);
            for (k, l) in lines.iter().enumerate() {
                self.need(lh);
                if k == 0 {
                    self.rect(ML + 1.0, self.y - pt * PT + 0.6, 1.4, 1.4, accent());
                }
                self.text_at(l, pt, ML + 5.0, self.y - pt * PT, &self.font.clone(), grey(0.12));
                self.y -= lh;
            }
            self.y -= 0.8;
        }
        self.y -= 1.0;
    }

    fn h1(&mut self, title: &str) {
        self.need(30.0);
        self.y -= 4.0;
        self.toc.push((title.to_string(), self.pages.len()));
        self.rect(ML, self.y - 9.0, 3.0, 9.0, accent());
        self.text_at(title, 16.0, ML + 6.0, self.y - 7.0, &self.bold.clone(), navy());
        self.y -= 11.0;
        self.line(ML, self.y, W - MR, self.y, grey(0.75), 0.4);
        self.y -= 5.0;
    }

    fn h2(&mut self, title: &str) {
        self.need(16.0);
        self.y -= 2.0;
        self.text_at(title, 11.5, ML, self.y - 4.5, &self.bold.clone(), navy());
        self.y -= 8.0;
    }

    /// Two-column key/value block with wrapping values.
    fn kv(&mut self, rows: &[(String, String)]) {
        let pt = 9.5;
        let lh = pt * PT * 1.5;
        let key_w = 52.0;
        for (i, (k, v)) in rows.iter().enumerate() {
            let lines = Self::wrap(v, pt, CW - key_w - 4.0);
            let h = lh * lines.len() as f32 + 1.6;
            self.need(h);
            if i % 2 == 0 {
                self.rect(ML, self.y - h, CW, h, grey(0.95));
            }
            self.text_at(k, pt, ML + 2.0, self.y - pt * PT - 0.8, &self.bold.clone(), grey(0.25));
            for (j, l) in lines.iter().enumerate() {
                self.text_at(l, pt, ML + key_w, self.y - pt * PT - 0.8 - j as f32 * lh, &self.font.clone(), grey(0.08));
            }
            self.y -= h;
        }
        self.y -= 3.0;
    }

    /// Monospaced table; widths are in characters.
    fn table(&mut self, header: &[&str], widths: &[usize], rows: &[Vec<String>], pt: f32) {
        let char_w = pt * PT * 0.6;
        let lh = pt * PT * 1.55;
        let fit = |s: &str, w: usize| {
            let s = clean(s);
            if s.chars().count() <= w { format!("{s:<w$}") } else { format!("{}~", s.chars().take(w.saturating_sub(1)).collect::<String>()) }
        };
        let line_of = |cells: &[String]| cells.iter().zip(widths).map(|(c, &w)| fit(c, w)).collect::<Vec<_>>().join(" ");
        let total_w = (widths.iter().sum::<usize>() + widths.len()) as f32 * char_w + 3.0;
        let head = line_of(&header.iter().map(|s| s.to_string()).collect::<Vec<_>>());
        let draw_head = |d: &mut Doc| {
            d.rect(ML, d.y - lh, total_w.min(CW), lh, navy());
            d.text_at(&head, pt, ML + 1.5, d.y - lh + lh * 0.3, &d.mono.clone(), grey(1.0));
            d.y -= lh;
        };
        self.need(lh * 3.0);
        draw_head(self);
        for (i, r) in rows.iter().enumerate() {
            if self.y - lh < BOTTOM {
                self.page();
                draw_head(self);
            }
            if i % 2 == 1 {
                self.rect(ML, self.y - lh, total_w.min(CW), lh, grey(0.94));
            }
            self.text_at(&line_of(r), pt, ML + 1.5, self.y - lh + lh * 0.3, &self.mono.clone(), grey(0.08));
            self.y -= lh;
        }
        self.y -= 4.0;
    }

    /// Stat cards in a row: (value, label).
    fn cards(&mut self, items: &[(String, String)]) {
        let n = items.len().max(1) as f32;
        let gap = 3.0;
        let w = (CW - gap * (n - 1.0)) / n;
        let h = 18.0;
        self.need(h + 4.0);
        for (i, (v, l)) in items.iter().enumerate() {
            let x = ML + i as f32 * (w + gap);
            self.rect(x, self.y - h, w, h, grey(0.95));
            self.rect(x, self.y - h, 1.2, h, accent());
            self.text_at(v, 13.0, x + 4.0, self.y - 8.0, &self.bold.clone(), navy());
            self.text_at(l, 7.5, x + 4.0, self.y - 14.0, &self.font.clone(), grey(0.35));
        }
        self.y -= h + 5.0;
    }

    fn bar_chart(&mut self, data: &[(String, f64)], max_val: f64, unit: &str) {
        if data.is_empty() {
            return;
        }
        let h = 52.0;
        let axis_w = 12.0;
        self.need(h + 16.0);
        let top = self.y;
        let base = top - h;
        let plot_w = CW - axis_w;
        let bw = plot_w / data.len() as f32;
        for k in 0..=4 {
            let yy = base + h * k as f32 / 4.0;
            self.line(ML + axis_w, yy, W - MR, yy, grey(0.85), 0.2);
            self.text_at(&format!("{:.0}", max_val * k as f64 / 4.0), 6.5, ML, yy - 1.0, &self.font.clone(), grey(0.4));
        }
        for (i, (label, v)) in data.iter().enumerate() {
            let bh = if max_val > 0.0 { (v / max_val) as f32 * h } else { 0.0 };
            let x = ML + axis_w + i as f32 * bw;
            let col = if *v >= max_val * 0.98 { navy() } else if *v > 0.0 { rgb(0.85, 0.55, 0.20) } else { grey(0.8) };
            self.rect(x + bw * 0.12, base, bw * 0.76, bh.max(0.3), col);
            let every = (data.len() / 12).max(1);
            if i % every == 0 {
                self.text_at(label, 6.0, x, base - 4.0, &self.font.clone(), grey(0.35));
            }
        }
        self.text_at(unit, 7.0, ML + axis_w, top + 2.0, &self.font.clone(), grey(0.35));
        self.y = base - 9.0;
    }

    /// Horizontal disk map: (label, fraction start, fraction width, colour).
    fn disk_map(&mut self, parts: &[(String, f32, f32, Color)]) {
        let h = 12.0;
        self.need(h + 14.0);
        self.rect(ML, self.y - h, CW, h, grey(0.88));
        for (label, a, wf, c) in parts {
            let x = ML + CW * a;
            let w = (CW * wf).max(0.8);
            self.rect(x, self.y - h, w, h, c.clone());
            if w > 22.0 {
                self.text_at(label, 7.5, x + 2.0, self.y - h / 2.0 - 1.2, &self.bold.clone(), grey(1.0));
            }
        }
        self.y -= h + 3.0;
        for (label, _, _, c) in parts {
            self.need(5.0);
            self.rect(ML, self.y - 3.0, 3.0, 3.0, c.clone());
            self.text_at(label, 8.0, ML + 5.0, self.y - 2.8, &self.font.clone(), grey(0.2));
            self.y -= 4.6;
        }
        self.y -= 3.0;
    }

    fn image(&mut self, jpeg: &[u8], x: f32, y_top: f32, width: f32) -> f32 {
        let Ok(img) = image_crate::load_from_memory(jpeg) else { return 0.0 };
        let rgb = img.to_rgb8();
        let (pw, ph) = (rgb.width() as f32, rgb.height() as f32);
        if pw < 1.0 {
            return 0.0;
        }
        let height = width * ph / pw;
        let dpi = pw * 25.4 / width;
        Image::from_dynamic_image(&DynamicImage::ImageRgb8(rgb)).add_to_layer(
            self.layer.clone(),
            ImageTransform { translate_x: Some(Mm(x)), translate_y: Some(Mm(y_top - height)), dpi: Some(dpi), ..Default::default() },
        );
        height
    }

    fn gallery(&mut self, thumbs: &[Thumbnail]) {
        let cols = 2usize;
        let gap = 6.0;
        let w = (CW - gap * (cols as f32 - 1.0)) / cols as f32;
        for row in thumbs.chunks(cols) {
            // 16:9-ish frames; reserve enough room for image + caption.
            let need = w * 0.62 + 14.0;
            self.need(need);
            let top = self.y;
            let mut max_h: f32 = 0.0;
            for (i, t) in row.iter().enumerate() {
                let x = ML + i as f32 * (w + gap);
                let h = self.image(&t.jpeg, x, top, w);
                let h = if h <= 0.0 {
                    self.rect(x, top - w * 0.56, w, w * 0.56, grey(0.9));
                    self.text_at("(frame could not be decoded)", 8.0, x + 4.0, top - w * 0.3, &self.font.clone(), grey(0.4));
                    w * 0.56
                } else {
                    h
                };
                let mut cy = top - h - 3.5;
                for l in Self::wrap(&t.caption, 7.2, w).iter().take(3) {
                    self.text_at(l, 7.2, x, cy, &self.font.clone(), grey(0.25));
                    cy -= 3.2;
                }
                max_h = max_h.max(h + 3.5 + 3.2 * 3.0);
            }
            self.y = top - max_h - 4.0;
        }
    }

    fn finish(self, case_id: &str, out: &Path, toc_page: usize) -> Result<(), ReportError> {
        let total = self.pages.len();
        // Table of contents (reserved page).
        if let Some(&(p, l)) = self.pages.get(toc_page - 1) {
            let layer = self.doc.get_page(p).get_layer(l);
            let mut y = H - TOP - 18.0;
            for (title, page) in &self.toc {
                layer.set_fill_color(grey(0.1));
                layer.use_text(clean(title), 11.0, Mm(ML + 4.0), Mm(y), &self.font);
                let dots = ".".repeat(((CW - 30.0 - title.len() as f32 * 11.0 * PT * 0.5) / 1.6).max(2.0) as usize);
                layer.set_fill_color(grey(0.6));
                layer.use_text(dots, 11.0, Mm(ML + 6.0 + title.len() as f32 * 11.0 * PT * 0.5), Mm(y), &self.font);
                layer.set_fill_color(navy());
                layer.use_text(page.to_string(), 11.0, Mm(W - MR - 8.0), Mm(y), &self.bold);
                y -= 8.0;
            }
        }
        // Running header and footer on every page except the cover.
        for (i, &(p, l)) in self.pages.iter().enumerate().skip(1) {
            let layer = self.doc.get_page(p).get_layer(l);
            layer.set_fill_color(grey(0.45));
            layer.use_text(clean(&format!("SATYA forensic examination report  |  Case {case_id}")), 8.0, Mm(ML), Mm(H - 12.0), &self.font);
            layer.use_text("CONFIDENTIAL - FORENSIC WORK PRODUCT", 8.0, Mm(W - MR - 62.0), Mm(H - 12.0), &self.bold);
            layer.set_outline_color(grey(0.8));
            layer.set_outline_thickness(0.3);
            layer.add_line(Line {
                points: vec![(Point::new(Mm(ML), Mm(H - 14.5)), false), (Point::new(Mm(W - MR), Mm(H - 14.5)), false)],
                is_closed: false,
            });
            layer.add_line(Line {
                points: vec![(Point::new(Mm(ML), Mm(14.0)), false), (Point::new(Mm(W - MR), Mm(14.0)), false)],
                is_closed: false,
            });
            layer.set_fill_color(grey(0.45));
            layer.use_text(format!("Page {} of {}", i + 1, total), 8.0, Mm(W - MR - 22.0), Mm(9.0), &self.font);
            layer.use_text("Generated by SATYA - read-only analysis", 8.0, Mm(ML), Mm(9.0), &self.font);
        }
        let f = std::fs::File::create(out)?;
        self.doc.save(&mut BufWriter::new(f)).map_err(|e| ReportError::Pdf(e.to_string()))?;
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Report content
// ---------------------------------------------------------------------------

pub fn generate(r: &ForensicReport, out: &Path) -> Result<(), ReportError> {
    let mut d = Doc::new(&format!("SATYA forensic report - {}", r.case_id))?;
    cover(&mut d, r);
    d.page();
    let toc_page = d.pages.len();
    d.text_at("Contents", 18.0, ML, H - TOP - 6.0, &d.bold.clone(), navy());
    d.page();

    d.h1("1. Executive summary");
    d.cards(&[
        (human_secs(r.recording.recorded_seconds), "recorded video".into()),
        (r.recording.recorded.to_string(), "recording slots".into()),
        (r.deleted.len().to_string(), "deleted entries".into()),
        (
            match &r.evidence.verified {
                Some(v) if v.md5_matches == Some(true) => "VERIFIED".into(),
                Some(v) if v.md5_matches == Some(false) => "MISMATCH".into(),
                Some(_) => "hashed".into(),
                None => "stored only".into(),
            },
            "evidence integrity".into(),
        ),
    ]);
    d.bullets(&r.findings);
    if !r.warnings.is_empty() {
        d.h2("Points requiring the examiner's attention");
        d.bullets(&r.warnings);
    }

    d.h1("2. Case information");
    let mut rows = vec![
        ("Case ID".into(), r.case_id.clone()),
        ("Case name".into(), r.case_name.clone()),
        ("Examiner".into(), or_dash(&r.examiner)),
        ("Organisation".into(), or_dash(&r.organization)),
        ("Report generated (UTC)".into(), r.generated_utc.clone()),
        ("Analysis tool".into(), r.tool.clone()),
    ];
    if !r.notes.trim().is_empty() {
        rows.push(("Examiner notes".into(), r.notes.clone()));
    }
    d.kv(&rows);

    evidence_section(&mut d, r);
    device_section(&mut d, r);
    recording_section(&mut d, r);
    if !r.thumbnails.is_empty() {
        d.h1("6. Visual evidence");
        d.para(
            "Frames below were decoded directly from the recording slots inside the image, \
             sampled evenly across the recorded period. Times are the recorder's own header \
             clock, shown as UTC; see Section 7 for how far that clock can be trusted.",
        );
        d.gallery(&r.thumbnails);
    }
    timestamp_section(&mut d, r);
    deleted_section(&mut d, r);
    exports_section(&mut d, r);
    method_section(&mut d, r);
    activity_section(&mut d, r);
    certificate(&mut d, r);
    appendix(&mut d, r);

    d.finish(&r.case_id, out, toc_page)
}

fn or_dash(s: &str) -> String {
    if s.trim().is_empty() { "-".into() } else { s.to_string() }
}

fn cover(d: &mut Doc, r: &ForensicReport) {
    d.rect(0.0, H - 95.0, W, 95.0, navy());
    d.rect(0.0, H - 97.0, W, 2.0, accent());
    d.text_at("SATYA", 34.0, ML, H - 38.0, &d.bold.clone(), grey(1.0));
    d.text_at("Forensic Examination Report", 18.0, ML, H - 52.0, &d.font.clone(), grey(0.92));
    d.text_at("Digital video recorder evidence", 11.0, ML, H - 62.0, &d.font.clone(), rgb(0.75, 0.82, 0.95));
    d.text_at(&format!("Case {}", r.case_id), 13.0, ML, H - 80.0, &d.bold.clone(), grey(1.0));
    d.y = H - 115.0;
    let ev_name = Path::new(&r.evidence.path).file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    d.kv(&[
        ("Case name".into(), r.case_name.clone()),
        ("Evidence item".into(), ev_name),
        ("Device".into(), format!("{} ({})", r.device.family, r.device.model)),
        ("Media size".into(), format!("{} ({} bytes)", human_bytes(r.evidence.media_bytes), r.evidence.media_bytes)),
        ("Acquisition MD5 (stored)".into(), r.evidence.stored_md5.clone().unwrap_or_else(|| "-".into())),
        ("Examiner".into(), or_dash(&r.examiner)),
        ("Organisation".into(), or_dash(&r.organization)),
        ("Generated (UTC)".into(), r.generated_utc.clone()),
    ]);
    d.y -= 6.0;
    d.para_sized(
        "This report was produced by SATYA from a read-only analysis of the evidence image. \
         Nothing in the image was modified. Findings are reproducible by re-running the same \
         tool version on an image with the same hash.",
        9.0,
        grey(0.35),
    );
}

fn evidence_section(d: &mut Doc, r: &ForensicReport) {
    let e = &r.evidence;
    d.h1("3. Evidence and integrity");
    d.kv(&[
        ("Image file".into(), e.path.clone()),
        ("Container format".into(), e.container.to_uppercase()),
        ("Logical media size".into(), format!("{} bytes ({}, {} sectors of 512)", e.media_bytes, human_bytes(e.media_bytes), e.media_bytes / 512)),
        ("Stored MD5".into(), e.stored_md5.clone().unwrap_or_else(|| "none stored".into())),
        ("Stored SHA-1".into(), e.stored_sha1.clone().unwrap_or_else(|| "none stored".into())),
        ("Acquisition read errors".into(), if e.read_errors == 0 { "none recorded".into() } else { format!("{} sector ranges", e.read_errors) }),
    ]);
    d.h2("Integrity verification");
    match &e.verified {
        Some(v) => {
            let verdict = |m: Option<bool>| match m {
                Some(true) => "MATCHES the acquisition hash",
                Some(false) => "DOES NOT MATCH the acquisition hash",
                None => "no stored hash to compare",
            };
            d.kv(&[
                ("Computed MD5".into(), format!("{}  ({})", v.md5, verdict(v.md5_matches))),
                ("Computed SHA-1".into(), format!("{}  ({})", v.sha1, verdict(v.sha1_matches))),
                ("Computed SHA-256".into(), v.sha256.clone()),
            ]);
        }
        None => d.para(
            "The full logical media was not re-hashed in this session. The hashes above are the \
             values the imaging tool stored in the E01. To verify, use Tools > Verify image \
             integrity (or `satya-cli verify`) and regenerate this report.",
        ),
    }
    if !e.acquisition.is_empty() {
        d.h2("Acquisition details (recorded in the E01 by the imaging tool)");
        d.kv(&e.acquisition);
    }
    if !e.segments.is_empty() {
        d.h2("Container segment files");
        d.para_sized(
            "SHA-256 of each segment file as stored on disk. These identify the container files; \
             the evidence hash is the media hash above.",
            8.5,
            grey(0.35),
        );
        let rows: Vec<Vec<String>> = e
            .segments
            .iter()
            .map(|(p, b, h)| vec![Path::new(p).file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default(), human_bytes(*b), h.clone()])
            .collect();
        d.table(&["Segment", "Size", "SHA-256"], &[24, 11, 64], &rows, 7.0);
    }
}

fn device_section(d: &mut Doc, r: &ForensicReport) {
    d.h1("4. Device identification and disk layout");
    d.kv(&[
        ("Recorder family".into(), r.device.family.clone()),
        ("Model / brands".into(), r.device.model.clone()),
        ("Manufacturer".into(), r.device.manufacturer.clone()),
        ("Identification confidence".into(), format!("{:.0}%", r.device.confidence * 100.0)),
    ]);
    d.h2("Evidence used for identification");
    d.bullets(&r.device.signals);
    d.h2("Partition layout");
    let total = r.disk_bytes.max(1) as f32;
    let palette = [navy(), rgb(0.20, 0.55, 0.45), rgb(0.85, 0.55, 0.20), rgb(0.5, 0.35, 0.65)];
    let map: Vec<(String, f32, f32, Color)> = r
        .partitions
        .iter()
        .enumerate()
        .map(|(i, p)| {
            (
                format!("#{} {} {} ({})", p.index, p.fs, p.role, human_bytes(p.bytes)),
                (p.start_lba * 512) as f32 / total,
                p.bytes as f32 / total,
                palette[i % palette.len()].clone(),
            )
        })
        .collect();
    d.disk_map(&map);
    let rows: Vec<Vec<String>> = r
        .partitions
        .iter()
        .map(|p| vec![p.index.to_string(), p.fs.clone(), p.start_lba.to_string(), human_bytes(p.bytes), p.role.clone(), p.detail.clone()])
        .collect();
    d.table(&["#", "Filesystem", "Start LBA", "Size", "Role", "Details"], &[3, 10, 12, 11, 8, 46], &rows, 7.5);
}

fn recording_section(d: &mut Doc, r: &ForensicReport) {
    let s = &r.recording;
    d.h1("5. Recordings");
    d.cards(&[
        (s.recorded.to_string(), format!("of {} slots used", s.slots_total)),
        (human_secs(s.recorded_seconds), "total duration".into()),
        (human_bytes(s.payload_bytes), "video payload".into()),
        (s.est_fps.map(|f| format!("{f:.1}")).unwrap_or_else(|| "-".into()), "frames/s (est.)".into()),
    ]);
    d.kv(&[
        ("First recording starts".into(), s.first_utc.clone()),
        ("Last recording ends".into(), s.last_utc.clone()),
        ("Slot size".into(), if s.slot_bytes > 0 { human_bytes(s.slot_bytes) } else { "-".into() }),
        ("Slots: recorded / empty / residual / unrecognised".into(), format!("{} / {} / {} / {}", s.recorded, s.empty, s.residual, s.unrecognized)),
        ("Slots written in time order".into(), if s.monotonic { "yes".into() } else { "no (ring buffer wrapped, or several cameras)".into() }),
        ("Overlapping slots".into(), s.overlaps.to_string()),
        ("Gaps longer than 5 s".into(), s.gaps.len().to_string()),
    ]);
    if !s.hourly.is_empty() {
        d.h2("Recording coverage per hour (header clock, UTC)");
        d.bar_chart(&s.hourly, 60.0, "minutes recorded");
    }
    if !s.gaps.is_empty() {
        d.h2("Gaps between consecutive recordings");
        let rows: Vec<Vec<String>> = s.gaps.iter().take(40).map(|(slot, g)| vec![slot.to_string(), human_secs(*g)]).collect();
        d.table(&["Before slot", "Gap"], &[12, 20], &rows, 8.0);
    }
    d.h2("Video stream");
    d.kv(&[
        ("Codec".into(), "H.265 / HEVC (Annex B), verified from NAL headers".into()),
        ("Distinct stream configurations (SPS)".into(), format!("{}{}", s.distinct_sps, if s.distinct_sps > 1 { " - slots may hold more than one camera or profile" } else { "" })),
        ("NAL units".into(), format!("{} total, {} non-standard ({:.2}%)", s.nal_total, s.nal_nonstandard, pct(s.nal_nonstandard, s.nal_total))),
        ("Pictures".into(), s.pictures.to_string()),
        ("Header length (bytes -> slots)".into(), join_hist(&s.header_len)),
    ]);
}

fn timestamp_section(d: &mut Doc, r: &ForensicReport) {
    d.h1("7. Timestamp analysis");
    d.para(
        "Each recording slot carries two independent records of time: the recorder's slot header \
         (Unix seconds, written by the recorder's clock) and the FAT32 directory entry (written \
         by the same clock, but stored as local time without a timezone). Agreement between them \
         shows internal consistency only; it does not prove the recorder's clock was correct. An \
         external anchor (e.g. a known event visible on camera, or NTP logs) is needed for that.",
    );
    if !r.recording.fat_delta.is_empty() {
        d.h2("FAT directory time minus slot header end time");
        let rows: Vec<Vec<String>> = r.recording.fat_delta.iter().take(12).map(|(s, n)| vec![format!("{s:+} s"), n.to_string()]).collect();
        d.table(&["Difference", "Slots"], &[14, 10], &rows, 8.0);
        let (top, _) = r.recording.fat_delta.first().cloned().unwrap_or((0, 0));
        d.para(&format!(
            "The most common difference is {top:+} s. A difference near zero means the recorder's \
             local time was set to UTC (or the listing treated it as UTC); a difference of whole \
             hours indicates the recorder's timezone offset. FAT stores modification times in \
             2-second steps, which explains differences of -1 s."
        ));
    }
}

fn deleted_section(d: &mut Doc, r: &ForensicReport) {
    d.h1("8. Deleted and recoverable data");
    if r.deleted.is_empty() {
        d.para("No deleted directory entries were found in the browsable filesystems.");
    } else {
        d.para(&format!("{} deleted entr{} found in the image's filesystems:", r.deleted.len(), if r.deleted.len() == 1 { "y was" } else { "ies were" }));
        let rows: Vec<Vec<String>> = r
            .deleted
            .iter()
            .map(|x| vec![x.location.clone(), x.path.clone(), x.id.clone(), human_bytes(x.size), x.deleted_at.clone(), x.status.clone()])
            .collect();
        d.table(&["Where", "Path", "Id", "Size", "Deleted (UTC)", "Content"], &[9, 26, 12, 9, 19, 22], &rows, 7.0);
    }
    let s = &r.recording;
    d.h2("Recording slots without a header");
    d.para(&if s.residual > 0 {
        format!("{} slot(s) hold video data but no slot header. They are recovery candidates and can be exported with the residual option.", s.residual)
    } else {
        "No slot holds video without a header.".to_string()
    });
}

fn exports_section(d: &mut Doc, r: &ForensicReport) {
    d.h1("9. Exported files");
    if r.exports.is_empty() {
        d.para("No files were exported in this session.");
        return;
    }
    d.para("Files produced from the evidence during this session, with their hashes:");
    let rows: Vec<Vec<String>> = r.exports.iter().map(|x| vec![x.file.clone(), human_bytes(x.bytes), x.md5.clone(), x.sha256.clone()]).collect();
    d.table(&["File", "Size", "MD5", "SHA-256"], &[22, 10, 32, 34], &rows, 6.2);
}

fn method_section(d: &mut Doc, r: &ForensicReport) {
    d.h1("10. Methodology, tools and limitations");
    d.bullets(&[
        "The image was opened read-only; E01 segments were decompressed in memory and never written.".into(),
        "The partition table (GPT/MBR) was parsed, and the FAT32 and ext filesystems were read with SATYA's own read-only parsers.".into(),
        "Every recording slot (dirNNNNN/fileNNNN.dat) was classified by content: header present, empty, header-less data, or unknown.".into(),
        "Video payloads were located after each slot header and validated NAL unit by NAL unit as H.265.".into(),
        "Every exported byte range is traceable to absolute disk offsets (see the appendix and the export manifests).".into(),
        format!("Tool: {}.", r.tool),
    ]);
    if !r.limitations.is_empty() {
        d.h2("Limitations");
        d.bullets(&r.limitations);
    }
}

fn activity_section(d: &mut Doc, r: &ForensicReport) {
    d.h1("11. Activity log (this session)");
    d.para(
        "Each action below is chained to the previous one by SHA-256 and signed with an Ed25519 key \
         created when SATYA started. Changing any entry breaks every later hash.",
    );
    if !r.session_public_key.is_empty() {
        d.kv(&[("Session public key".into(), r.session_public_key.clone())]);
    }
    if r.activity.is_empty() {
        d.para("No actions were logged.");
    } else {
        let rows: Vec<Vec<String>> = r.activity.iter().map(|a| vec![a.time.clone(), a.action.clone(), a.detail.clone(), a.hash.clone()]).collect();
        d.table(&["Time (UTC)", "Action", "Detail", "Entry hash"], &[19, 10, 48, 18], &rows, 6.8);
    }
}

fn certificate(d: &mut Doc, r: &ForensicReport) {
    d.page();
    d.h1("12. Examiner's certificate (draft)");
    d.para_sized(
        "Draft for the examiner to review, complete and sign, for example as the basis of a \
         certificate under Section 63(4) of the Bharatiya Sakshya Adhiniyam, 2023. SATYA fills in \
         only facts it established itself; everything else must be completed by the examiner.",
        9.0,
        grey(0.35),
    );
    let hash_line = match &r.evidence.verified {
        Some(v) => format!("SHA-256 {} (MD5 {}), computed by SATYA over the full logical media", v.sha256, v.md5),
        None => format!(
            "MD5 {} as stored in the E01 by the imaging tool (not re-computed by SATYA in this session)",
            r.evidence.stored_md5.clone().unwrap_or_else(|| "-".into())
        ),
    };
    d.para(&format!(
        "I, ______________________ (name), ______________________ (designation), certify that the \
         electronic record identified as \"{}\" ({} bytes) has the hash value {}. The record was \
         examined read-only using SATYA ({}) on {}. The acquisition of the original storage medium \
         was performed by ______________________ on ______________ using ______________________.",
        Path::new(&r.evidence.path).file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default(),
        r.evidence.media_bytes,
        hash_line,
        r.tool,
        r.generated_utc
    ));
    d.y -= 14.0;
    for label in ["Signature", "Name", "Designation", "Date and place"] {
        d.need(12.0);
        d.line(ML + 40.0, d.y - 1.0, ML + 120.0, d.y - 1.0, grey(0.4), 0.3);
        d.text_at(label, 9.0, ML, d.y, &d.font.clone(), grey(0.3));
        d.y -= 12.0;
    }
}

fn appendix(d: &mut Doc, r: &ForensicReport) {
    if r.slots.is_empty() {
        return;
    }
    d.page();
    d.h1("Appendix A. Recording slot index");
    d.para_sized(
        "Every recorded slot in time order. Sector = absolute 512-byte sector where the slot's video \
         payload begins on the disk.",
        8.5,
        grey(0.35),
    );
    let rows: Vec<Vec<String>> = r
        .slots
        .iter()
        .map(|s| vec![s.slot.to_string(), s.path.clone(), s.start.clone(), s.end.clone(), s.seconds.to_string(), human_bytes(s.bytes), s.sector.to_string()])
        .collect();
    d.table(&["Slot", "File", "Start (UTC)", "End (UTC)", "Sec", "Payload", "Sector"], &[5, 22, 19, 19, 4, 10, 11], &rows, 6.6);
}

fn pct(a: u64, b: u64) -> f64 {
    if b == 0 { 0.0 } else { a as f64 * 100.0 / b as f64 }
}

fn join_hist<K: std::fmt::Display>(v: &[(K, usize)]) -> String {
    if v.is_empty() {
        return "-".into();
    }
    v.iter().take(6).map(|(k, n)| format!("{k} -> {n}")).collect::<Vec<_>>().join(", ")
}
