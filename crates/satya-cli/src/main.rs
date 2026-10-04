mod evidence;

use clap::{Parser, Subcommand};
use std::io::Write;
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "satya")]
#[command(about = "Multi-vendor DVR/NVR forensic analysis tool")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Identify the OEM of a disk image
    Identify {
        #[arg(short, long)]
        image: PathBuf,
    },
    /// Enumerate all frames (allocated + carved)
    Enumerate {
        #[arg(short, long)]
        image: PathBuf,
        #[arg(short, long)]
        output: Option<PathBuf>,
    },
    /// Hash a file with SHA-256
    Hash {
        #[arg(short, long)]
        image: PathBuf,
    },
    /// Extract raw H.264 NAL units to a .h264 file
    Export {
        #[arg(short, long)]
        image: PathBuf,
        #[arg(short, long)]
        output: PathBuf,
        /// Also write a JSON sidecar with frame metadata
        #[arg(long, default_value_t = false)]
        metadata: bool,
    },
    /// Show container info, stored hashes and partitions (streams; works on E01)
    ImageInfo {
        #[arg(short, long)]
        image: PathBuf,
    },
    /// Hash the full media (MD5, SHA-1, SHA-256) and compare with hashes stored in an E01
    Verify {
        #[arg(short, long)]
        image: PathBuf,
        /// Write the result as JSON
        #[arg(short, long)]
        output: Option<PathBuf>,
    },
    /// Scan a JUAN (HeimVision/Zosi K9604-W family) volume and check the format
    JuanScan {
        #[arg(short, long)]
        image: PathBuf,
        /// Write every slot record as JSON
        #[arg(short, long)]
        output: Option<PathBuf>,
        /// Read header-less slots completely instead of sampling them
        #[arg(long, default_value_t = false)]
        deep: bool,
    },
    /// Export JUAN recordings as an H.265 stream plus a provenance manifest
    JuanExport {
        #[arg(short, long)]
        image: PathBuf,
        /// Output .hevc file (manifest is written next to it)
        #[arg(short, long)]
        output: PathBuf,
        /// Keep only NAL units with a valid H.265 header
        #[arg(long, default_value_t = false)]
        strip: bool,
        /// First slot number to include
        #[arg(long)]
        from: Option<u32>,
        /// Last slot number to include
        #[arg(long)]
        to: Option<u32>,
        /// Also export header-less slots that still hold video (recovery)
        #[arg(long, default_value_t = false)]
        residual: bool,
    },
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();
    match cli.command {
        Commands::Identify { image } => cmd_identify(&image)?,
        Commands::Enumerate { image, output } => cmd_enumerate(&image, output.as_ref())?,
        Commands::Hash { image } => cmd_hash(&image)?,
        Commands::Export { image, output, metadata } => {
            cmd_export(&image, &output, metadata)?
        }
        Commands::ImageInfo { image } => evidence::cmd_image_info(&image)?,
        Commands::Verify { image, output } => evidence::cmd_verify(&image, output.as_ref())?,
        Commands::JuanScan { image, output, deep } => evidence::cmd_juan_scan(&image, output.as_ref(), deep)?,
        Commands::JuanExport { image, output, strip, from, to, residual } => {
            evidence::cmd_juan_export(&image, &output, strip, from, to, residual)?
        }
    }
    Ok(())
}

fn cmd_identify(image: &PathBuf) -> Result<(), Box<dyn std::error::Error>> {
    // Streaming detectors first: they work on E01 and on images larger than RAM.
    let src = satya_image::open_image(image)?;
    if let Some(vol) = satya_parsers::juan::JuanVolume::open(src.as_ref())? {
        let fp = satya_parsers::juan::fingerprint(&vol.layout);
        println!("OEM:        {}", fp.oem.as_str());
        println!("Confidence: {:.2}", fp.confidence);
        for sig in &vol.layout.signals {
            println!("  - {sig}");
        }
        return Ok(());
    }
    let data = evidence::legacy_bytes(src.as_ref())?;
    match satya_parsers::identify_device(&data) {
        Some(fp) => {
            println!("OEM:        {}", fp.oem.as_str());
            println!("Confidence: {:.2}", fp.confidence);
            if let Some(bs) = fp.block_size {
                println!("Block size: {}", bs);
            }
        }
        None => println!("Unknown OEM"),
    }
    Ok(())
}

fn cmd_enumerate(
    image: &PathBuf,
    output: Option<&PathBuf>,
) -> Result<(), Box<dyn std::error::Error>> {
    let data = std::fs::read(image)?;
    let fp = satya_parsers::identify_device(&data).ok_or("Unknown OEM")?;
    println!("Identified: {}", fp.oem.as_str());

    let frames = satya_parsers::enumerate_frames(&data, fp.oem)?;
    println!("Frames found: {}", frames.len());

    let allocated = frames
    .iter()
    .filter(|f| matches!(f.recovery_source, satya_core::RecoverySource::Allocated))
    .count();
    let carved = frames.len() - allocated;
    println!("  Allocated: {}", allocated);
    println!("  Carved:    {}", carved);

    if let Some(out) = output {
        let json = serde_json::to_string_pretty(&frames)?;
        std::fs::write(out, json)?;
        println!("Wrote metadata → {}", out.display());
    }
    Ok(())
}

fn cmd_hash(image: &PathBuf) -> Result<(), Box<dyn std::error::Error>> {
    // Streams the logical media (for E01: the acquired disk, not the container file).
    let src = satya_image::open_image(image)?;
    let h = satya_image::hashing::hash_media(src.as_ref(), |_, _| {})?;
    println!("MD5:     {}", h.md5);
    println!("SHA-256: {}", h.sha256);
    Ok(())
}

fn cmd_export(
    image: &PathBuf,
    output: &PathBuf,
    write_metadata: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let data = std::fs::read(image)?;
    let fp = satya_parsers::identify_device(&data)
    .ok_or("Unknown OEM — cannot export")?;
    eprintln!("Identified OEM: {}", fp.oem.as_str());

    let frames = satya_parsers::enumerate_frames(&data, fp.oem)?;
    eprintln!("Found {} frames", frames.len());

    let mut file = std::fs::File::create(output)?;
    let mut written = 0usize;
    let mut total_bytes = 0usize;

    for frame in &frames {
        if frame.length == 0 {
            continue;
        }
        let start = frame.offset as usize;
        let end = (start + frame.length as usize).min(data.len());
        if start >= end {
            continue;
        }
        let bytes = &data[start..end];

        // Only write bytes that start with an H.264 NAL unit header.
        // Annex B: 00 00 00 01 or 00 00 01
        let has_nal = bytes.starts_with(&[0, 0, 0, 1])
        || bytes.starts_with(&[0, 0, 1]);
        if !has_nal {
            continue;
        }

        file.write_all(bytes)?;
        written += 1;
        total_bytes += bytes.len();
    }

    file.flush()?;
    eprintln!(
        "Wrote {} video frames → {} ({} bytes)",
              written,
              output.display(),
              total_bytes
    );

    if write_metadata {
        let meta_path = output.with_extension("frames.json");
        let json = serde_json::to_string_pretty(&frames)?;
        std::fs::write(&meta_path, json)?;
        eprintln!("Metadata → {}", meta_path.display());
    }

    Ok(())
}
