use clap::{Parser, Subcommand};
use sha2::{Digest, Sha256};
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
    }
    Ok(())
}

fn cmd_identify(image: &PathBuf) -> Result<(), Box<dyn std::error::Error>> {
    let data = std::fs::read(image)?;
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
    let data = std::fs::read(image)?;
    let mut hasher = Sha256::new();
    hasher.update(&data);
    println!("SHA-256: {}", hex::encode(hasher.finalize()));
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
