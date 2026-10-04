//! Validation against the real NIST CFReDS "Heimvision DVR .E01 Forensic Image".
//!
//! The image is not in the repo. Download it from cfreds.nist.gov, then run:
//!   SATYA_CFREDS_HEIM="/path/HeimVision K9604-W.E01" \
//!     cargo test -p satya-parsers --test cfreds_heimvision -- --ignored --nocapture
//!
//! Expected values come from sources independent of SATYA: the E01's own
//! acquisition metadata (ewfinfo) and the FTK Imager file listing shipped
//! with the dataset. Format claims not yet confirmed are printed, not asserted.

use satya_parsers::juan::{JuanVolume, ScanOptions};

const STORED_MD5: &str = "4895ea6d10b08c29fb1bb03591adc7b2";
const MEDIA_BYTES: u64 = 150_039_945_216;
const FAT_START_LBA: u64 = 10_485_760;
const SLOT_DIRS: usize = 134;
const SLOTS: usize = 17_152;
const SLOT_SIZE: u64 = 8_388_608;
/// Slots with FAT timestamps in the FTK listing (dir00000/file0000..dir00006/file0037).
const FAT_DATED: usize = 806;

#[test]
#[ignore = "needs the CFReDS image; set SATYA_CFREDS_HEIM"]
fn cfreds_heimvision_k9604w() {
    let Ok(path) = std::env::var("SATYA_CFREDS_HEIM") else {
        eprintln!("SATYA_CFREDS_HEIM not set; skipping");
        return;
    };
    let src = satya_image::open_image(std::path::Path::new(&path)).expect("open image");
    assert_eq!(src.kind(), "e01");
    assert_eq!(src.len(), MEDIA_BYTES);
    assert_eq!(src.stored_hashes().md5.as_deref(), Some(STORED_MD5));

    let vol = JuanVolume::open(src.as_ref()).expect("read").expect("JUAN layout detected");
    let l = &vol.layout;
    assert_eq!(l.fat_partition.start_lba, FAT_START_LBA);
    assert_eq!(l.slot_dirs, SLOT_DIRS);
    assert_eq!(l.slots, SLOTS);
    assert_eq!(l.slot_size, Some(SLOT_SIZE));
    assert!(l.ext_partition.is_some(), "ext3 system partition");

    let scan = vol
        .scan(&ScanOptions::default(), |d, t| {
            if d % 1024 == 0 {
                eprintln!("scanned {d}/{t}");
            }
        })
        .expect("scan");
    let m = &scan.summary;
    assert_eq!(m.fat_dated_slots, FAT_DATED);
    assert!(m.recorded > 0, "no 'luo ' slots found");

    // Reported for the format notes; promote to asserts once confirmed.
    eprintln!("recorded={} empty={} residual={} unrecognized={}", m.recorded, m.empty, m.residual, m.unrecognized);
    eprintln!("header span {:?} -> {:?}", m.first_start_utc, m.last_end_utc);
    eprintln!("header length hist {:?}", m.header_len_hist);
    eprintln!("FAT - header end hist {:?}", m.fat_delta_hist);
    eprintln!("distinct SPS {} | NAL {} nonstandard {} | est fps {:?}", m.distinct_sps, m.nal_total, m.nal_nonstandard, m.est_fps);
    eprintln!("monotonic {} overlaps {} gaps {:?}", m.slot_order_monotonic, m.overlaps, &m.gaps[..m.gaps.len().min(10)]);
    for d in m.dirs.iter().filter(|d| d.recorded > 0) {
        eprintln!("dir{:05}: {}/{} {:?} -> {:?}", d.dir, d.recorded, d.slots, d.first_start_utc, d.last_end_utc);
    }
}
