use clap::{Parser, Subcommand};
use sha2::{Digest, Sha256};
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
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();
    match cli.command {
        Commands::Identify { image } => {
            let data = std::fs::read(&image)?;
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
        }
        Commands::Enumerate { image, output } => {
            let data = std::fs::read(&image)?;
            let fp = satya_parsers::identify_device(&data)
            .ok_or("Unknown OEM")?;
            println!("Identified: {}", fp.oem.as_str());

            let frames = satya_parsers::enumerate_frames(&data, fp.oem)?;
            println!("Frames found: {}", frames.len());

            let allocated = frames.iter()
            .filter(|f| matches!(f.recovery_source, satya_core::RecoverySource::Allocated))
            .count();
            let carved = frames.len() - allocated;
            println!("  Allocated: {}", allocated);
            println!("  Carved:    {}", carved);

            if let Some(out) = output {
                let json = serde_json::to_string_pretty(&frames)?;
                std::fs::write(out, json)?;
            }
        }
        Commands::Hash { image } => {
            let data = std::fs::read(&image)?;
            let mut hasher = Sha256::new();
            hasher.update(&data);
            let hash = hasher.finalize();
            println!("SHA-256: {}", hex::encode(hash));
        }
    }
    Ok(())
}
