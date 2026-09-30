//! SATYA MCP server.
//!
//! Exposes SATYA's forensic capabilities as MCP tools so an AI agent
//! (Claude Desktop, Cursor, etc.) can drive the analysis conversationally.

use rmcp::{
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    model::{ServerCapabilities, ServerInfo},
    schemars, tool, tool_handler, tool_router, ServerHandler,
};
use satya_core::{RecoverySource, TimestampSource};
use satya_trust::{fuse_with_diagnostics, to_claim, Source};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct SatyaMcp {
    tool_router: ToolRouter<Self>,
}

// ---------------------------------------------------------------------------
// Tool request types
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct IdentifyRequest {
    /// Absolute path to the DVR disk image (.img / .dd)
    pub image_path: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct EnumerateRequest {
    pub image_path: String,
    #[serde(default = "default_true")]
    pub include_carved: bool,
    #[serde(default = "default_limit")]
    pub limit: usize,
}

fn default_true() -> bool { true }
fn default_limit() -> usize { 500 }

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct TimelineRequest {
    pub image_path: String,
    #[serde(default)]
    pub ntp_log: Option<String>,
    #[serde(default)]
    pub after_utc: Option<String>,
    #[serde(default)]
    pub before_utc: Option<String>,
    #[serde(default = "default_limit")]
    pub limit: usize,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct ExportRequest {
    pub image_path: String,
    #[serde(default)]
    pub output_path: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct HashRequest {
    pub image_path: String,
    #[serde(default = "default_sha256")]
    pub algorithm: String,
}

fn default_sha256() -> String { "sha256".into() }

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct ReportRequest {
    pub image_path: String,
    pub case_id: String,
    pub case_name: String,
    pub investigator: String,
    pub organization: String,
    #[serde(default)]
    pub output_pdf: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct CustodyVerifyRequest {
    pub bundle_json: String,
    pub message: String,
}

// ---------------------------------------------------------------------------
// Tool implementations
// ---------------------------------------------------------------------------

#[tool_router]
impl SatyaMcp {
    pub fn new() -> Self {
        Self { tool_router: Self::tool_router() }
    }

    #[tool(description = "Identify the OEM (Hikvision, Dahua, CP Plus, Honeywell, Uniview, TP-Link, Godrej, Matrix, WFS) of a DVR/NVR disk image by reading its file-system signature. Returns the OEM name and confidence.")]
    async fn identify_oem(&self, Parameters(req): Parameters<IdentifyRequest>) -> String {
        match std::fs::read(&req.image_path) {
            Ok(data) => match satya_parsers::identify_device(&data) {
                Some(fp) => serde_json::json!({
                    "oem": fp.oem.as_str(),
                                              "confidence": fp.confidence,
                                              "block_size": fp.block_size,
                                              "model": fp.model,
                                              "firmware": fp.firmware,
                }).to_string(),
                None => r#"{"error":"unknown OEM"}"#.into(),
            },
            Err(e) => format!(r#"{{"error":"cannot read image: {e}"}}"#),
        }
    }

    #[tool(description = "Enumerate all video frames in a DVR disk image, including deleted frames carved from unallocated space. Returns frame offsets, lengths, recovery source, and per-frame timestamp claims.")]
    async fn enumerate_frames(&self, Parameters(req): Parameters<EnumerateRequest>) -> String {
        let data = match std::fs::read(&req.image_path) {
            Ok(d) => d,
            Err(e) => return format!(r#"{{"error":"{e}"}}"#),
        };
        let fp = match satya_parsers::identify_device(&data) {
            Some(f) => f,
            None => return r#"{"error":"unknown OEM"}"#.into(),
        };
        let frames = match satya_parsers::enumerate_frames(&data, fp.oem) {
            Ok(f) => f,
            Err(e) => return format!(r#"{{"error":"{e}"}}"#),
        };

        let filtered: Vec<_> = frames.iter()
        .filter(|f| req.include_carved || matches!(f.recovery_source, RecoverySource::Allocated))
        .take(req.limit)
        .map(|f| serde_json::json!({
            "offset": f.offset,
            "length": f.length,
            "codec": format!("{:?}", f.codec),
                                   "recovery_source": format!("{:?}", f.recovery_source),
                                   "recovery_confidence": f.recovery_confidence,
                                   "claims": f.claims.iter().map(|c| serde_json::json!({
                                       "claimed_utc": c.claimed_utc.to_rfc3339(),
                                                                                       "source": format!("{:?}", c.source),
                                                                                       "confidence": c.confidence,
                                   })).collect::<Vec<_>>(),
        })).collect();

        serde_json::json!({
            "oem": fp.oem.as_str(),
                          "total_frames": frames.len(),
                          "returned": filtered.len(),
                          "frames": filtered,
        }).to_string()
    }

    #[tool(description = "Fuse all available timestamp sources (frame headers, device logs, NTP sync logs, on-screen OCR) into a per-frame posterior mean with a 95% confidence interval. Same-clock sources are collapsed before fusion to prevent false confidence. Optionally filter by UTC time window.")]
    async fn analyze_timeline(&self, Parameters(req): Parameters<TimelineRequest>) -> String {
        let data = match std::fs::read(&req.image_path) {
            Ok(d) => d,
            Err(e) => return format!(r#"{{"error":"{e}"}}"#),
        };
        let fp = match satya_parsers::identify_device(&data) {
            Some(f) => f,
            None => return r#"{"error":"unknown OEM"}"#.into(),
        };
        let frames = match satya_parsers::enumerate_frames(&data, fp.oem) {
            Ok(f) => f,
            Err(e) => return format!(r#"{{"error":"{e}"}}"#),
        };

        let ntp_events = req.ntp_log.as_deref()
        .map(satya_anchors::extract_ntp_events)
        .unwrap_or_default();

        let after = req.after_utc.as_deref().and_then(parse_iso);
        let before = req.before_utc.as_deref().and_then(parse_iso);

        let mut out = Vec::new();
        for (i, frame) in frames.iter().enumerate() {
            if out.len() >= req.limit { break; }

            let mut claims: Vec<_> = frame.claims.iter().map(|c| {
                let src = match c.source {
                    TimestampSource::FrameHeader => Source::FrameHeader,
                    TimestampSource::DeviceLog => Source::DeviceLog,
                    TimestampSource::OnScreenOcr => Source::OnScreenOcr,
                    TimestampSource::NtpSyncLog => Source::NtpSyncLog,
                    TimestampSource::SeiMetadata => Source::FrameHeader,
                };
                to_claim(c.claimed_utc.timestamp() as f64, src, c.confidence as f64)
            }).collect();

            if !ntp_events.is_empty() {
                if let Some(dvr_t) = frame.claims.iter()
                    .find(|c| matches!(c.source, TimestampSource::FrameHeader))
                    .map(|c| c.claimed_utc.timestamp() as f64)
                    {
                        let nearest = ntp_events.iter()
                        .filter(|e| e.dvr_time_s <= dvr_t)
                        .max_by(|a, b| a.dvr_time_s.partial_cmp(&b.dvr_time_s).unwrap());
                        if let Some(ev) = nearest {
                            claims.push(to_claim(ev.true_time_s, Source::NtpSyncLog, 0.95));
                        }
                    }
            }

            if claims.is_empty() { continue; }
            let ft = fuse_with_diagnostics(&claims);

            if let Some(a) = after { if ft.mean_s < a { continue; } }
            if let Some(b) = before { if ft.mean_s > b { continue; } }

            out.push(serde_json::json!({
                "frame_index": i,
                "offset": frame.offset,
                "mean_utc": ft.mean_iso(),
                                       "ci_lo_95_utc": chrono::DateTime::from_timestamp(ft.ci_lo_95_s as i64, 0)
                                       .map(|d| d.to_rfc3339()).unwrap_or_default(),
                                       "ci_hi_95_utc": chrono::DateTime::from_timestamp(ft.ci_hi_95_s as i64, 0)
                                       .map(|d| d.to_rfc3339()).unwrap_or_default(),
                                       "sigma_s": ft.sigma_s,
                                       "collapsed": ft.collapsed,
                                       "effective_sources": ft.effective_sources.iter()
                                       .map(|s| format!("{:?}", s)).collect::<Vec<_>>(),
            }));
        }

        serde_json::json!({
            "oem": fp.oem.as_str(),
                          "total_frames": frames.len(),
                          "fused": out.len(),
                          "ntp_events_used": ntp_events.len(),
                          "timeline": out,
        }).to_string()
    }

    #[tool(description = "Extract raw H.264 NAL units from a DVR image and wrap them into a browser-playable MP4 using FFmpeg. Returns the output path and byte size.")]
    async fn export_video(&self, Parameters(req): Parameters<ExportRequest>) -> String {
        let img = PathBuf::from(&req.image_path);
        let out_mp4 = req.output_path.map(PathBuf::from)
        .unwrap_or_else(|| img.with_extension("mp4"));
        let tmp_h264 = img.with_extension("h264");

        let data = match std::fs::read(&img) {
            Ok(d) => d,
            Err(e) => return format!(r#"{{"error":"{e}"}}"#),
        };
        let fp = match satya_parsers::identify_device(&data) {
            Some(f) => f,
            None => return r#"{"error":"unknown OEM"}"#.into(),
        };
        let frames = match satya_parsers::enumerate_frames(&data, fp.oem) {
            Ok(f) => f,
            Err(e) => return format!(r#"{{"error":"{e}"}}"#),
        };

        use std::io::Write;
        let mut f = match std::fs::File::create(&tmp_h264) {
            Ok(f) => f,
            Err(e) => return format!(r#"{{"error":"cannot write tmp: {e}"}}"#),
        };
        let mut written = 0usize;
        for frame in &frames {
            if frame.length == 0 { continue; }
            let s = frame.offset as usize;
            let e = (s + frame.length as usize).min(data.len());
            if s >= e { continue; }
            let bytes = &data[s..e];
            if bytes.starts_with(&[0, 0, 0, 1]) || bytes.starts_with(&[0, 0, 1]) {
                if f.write_all(bytes).is_err() { break; }
                written += 1;
            }
        }
        drop(f);

        if written == 0 { return r#"{"error":"no video frames extracted"}"#.into(); }

        match satya_video::h264_to_mp4(&tmp_h264, &out_mp4) {
            Ok(()) => {
                let size = std::fs::metadata(&out_mp4).map(|m| m.len()).unwrap_or(0);
                serde_json::json!({
                    "output_mp4": out_mp4.to_string_lossy(),
                                  "size_bytes": size,
                                  "frames_written": written,
                }).to_string()
            }
            Err(e) => format!(r#"{{"error":"ffmpeg: {e}"}}"#),
        }
    }

    #[tool(description = "Compute a cryptographic hash of a disk image for evidence integrity. Supports sha256 (default), md5, blake3. Returns the algorithm and hex digest.")]
    async fn hash_image(&self, Parameters(req): Parameters<HashRequest>) -> String {
        let data = match std::fs::read(&req.image_path) {
            Ok(d) => d,
            Err(e) => return format!(r#"{{"error":"{e}"}}"#),
        };
        let digest = match req.algorithm.as_str() {
            "sha256" => hex::encode(Sha256::digest(&data)),
            "md5" => {
                use md5::Md5;
                hex::encode(Md5::digest(&data))
            }
            "blake3" => blake3::hash(&data).to_hex().to_string(),
            other => return format!(r#"{{"error":"unsupported algorithm: {other}"}}"#),
        };
        serde_json::json!({
            "algorithm": req.algorithm,
            "digest": digest,
            "size_bytes": data.len(),
        }).to_string()
    }

    #[tool(description = "Generate a court-ready PDF report for a DVR image: evidence hashes, recovered frame counts, per-frame timestamp confidence intervals, and an evidence-integrity certificate. Returns the PDF path.")]
    async fn generate_report(&self, Parameters(req): Parameters<ReportRequest>) -> String {
        let data = match std::fs::read(&req.image_path) {
            Ok(d) => d,
            Err(e) => return format!(r#"{{"error":"{e}"}}"#),
        };
        let fp = match satya_parsers::identify_device(&data) {
            Some(f) => f,
            None => return r#"{"error":"unknown OEM"}"#.into(),
        };
        let frames = match satya_parsers::enumerate_frames(&data, fp.oem) {
            Ok(f) => f,
            Err(e) => return format!(r#"{{"error":"{e}"}}"#),
        };
        let sha256 = hex::encode(Sha256::digest(&data));
        let allocated = frames.iter()
        .filter(|f| matches!(f.recovery_source, RecoverySource::Allocated))
        .count();

        let mut timeline = Vec::new();
        for frame in &frames {
            let mut claims = Vec::new();
            for c in &frame.claims {
                let src = match c.source {
                    TimestampSource::FrameHeader => Source::FrameHeader,
                    TimestampSource::DeviceLog => Source::DeviceLog,
                    TimestampSource::OnScreenOcr => Source::OnScreenOcr,
                    TimestampSource::NtpSyncLog => Source::NtpSyncLog,
                    TimestampSource::SeiMetadata => Source::FrameHeader,
                };
                claims.push(to_claim(c.claimed_utc.timestamp() as f64, src, c.confidence as f64));
            }
            if claims.is_empty() { continue; }
            timeline.push(fuse_with_diagnostics(&claims));
        }

        let custody: Vec<satya_custody::Checkpoint> = Vec::new();

        let out = req.output_pdf.map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(&req.image_path).with_extension("report.pdf"));

        let summary = satya_parsers::analysis::build_summary(&data, &fp, &frames);

    let input = satya_report::ReportInput {
        case_id: req.case_id,
        case_name: req.case_name,
        investigator: req.investigator,
        organization: req.organization,
        evidence_path: req.image_path.clone(),
        oem: fp.oem.as_str().into(),
        sha256: sha256.clone(),
        md5: {
            use md5::Md5;
            hex::encode(Md5::digest(&data))
        },
        total_frames: frames.len(),
        allocated,
        carved: frames.len() - allocated,
        size_bytes: data.len() as u64,
        device: serde_json::to_value(&summary.device).unwrap_or(serde_json::json!({})),
        partitions: serde_json::to_value(&summary.partitions).unwrap_or(serde_json::json!([])),
        channels: serde_json::to_value(&summary.channels).unwrap_or(serde_json::json!([])),
        logs: serde_json::to_value(&summary.logs).unwrap_or(serde_json::json!([])),
        allocation: serde_json::to_value(&summary.allocation).unwrap_or(serde_json::json!([])),
        timeline,
        custody: Vec::new(),
    };

        match satya_report::generate_report(&input, &out) {
            Ok(()) => serde_json::json!({
                "output_pdf": out.to_string_lossy(),
                                        "size_bytes": std::fs::metadata(&out).map(|m| m.len()).unwrap_or(0),
            }).to_string(),
            Err(e) => format!(r#"{{"error":"{e}"}}"#),
        }
    }

    #[tool(description = "Verify a hybrid Ed25519 signature bundle over a custody checkpoint message. Returns true if the signature is valid, false otherwise.")]
    async fn verify_custody(&self, Parameters(req): Parameters<CustodyVerifyRequest>) -> String {
        let bundle: satya_custody::SignatureBundle = match serde_json::from_str(&req.bundle_json) {
            Ok(b) => b,
            Err(e) => return format!(r#"{{"error":"bad bundle json: {e}"}}"#),
        };
        let ok = satya_custody::verify(req.message.as_bytes(), &bundle);
        serde_json::json!({ "valid": ok }).to_string()
    }
}

fn parse_iso(s: &str) -> Option<f64> {
    chrono::DateTime::parse_from_rfc3339(s).ok().map(|dt| dt.timestamp() as f64)
}

#[tool_handler]
impl ServerHandler for SatyaMcp {
    fn get_info(&self) -> ServerInfo {
        ServerInfo {
            instructions: Some(
                "SATYA is a multi-vendor DVR/NVR forensic analysis tool. \
Use identify_oem first to detect the OEM of a disk image, \
then enumerate_frames to list video frames, analyze_timeline \
to obtain per-frame timestamp confidence intervals, and \
generate_report to produce a court-ready PDF. All tools are \
read-only with respect to the evidence image; they never \
modify the original bytes."
.into(),
            ),
            capabilities: ServerCapabilities::builder().enable_tools().build(),
            ..Default::default()
        }
    }
}

pub async fn serve_stdio() -> anyhow::Result<()> {
    use rmcp::{ServiceExt, transport::stdio};
    let service = SatyaMcp::new().serve(stdio()).await?;
    service.waiting().await?;
    Ok(())
}
