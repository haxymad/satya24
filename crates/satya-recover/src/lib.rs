use satya_core::*;
use satya_parsers::nal_scan;
use rayon::prelude::*;

/// Carve frames in parallel across image chunks.
pub fn carve_parallel(image: &[u8], chunk_size: usize) -> Vec<RecoveredFrame> {
    let chunks: Vec<&[u8]> = image.chunks(chunk_size).collect();
    let mut all: Vec<RecoveredFrame> = chunks
    .par_iter()
    .flat_map(|chunk| nal_scan::scan_nal_units(chunk))
    .collect();
    all.sort_by_key(|f| f.offset);
    all
}
