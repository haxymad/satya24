use satya_image::{open_image, vfs::{volumes, Volume}};
fn walk(v: &Volume, node: &str, depth: usize) {
    for e in v.list(node).unwrap() {
        println!("{}{} [{}] size={} del={} rec={} mod={:?} basis={} id={} off={:?} note={:?}", "  ".repeat(depth), e.name, e.node, e.size, e.deleted, e.recoverable, e.modified, e.time_basis, e.id, e.disk_offset, e.note);
        if e.is_dir && depth < 2 { walk(v, &e.node, depth + 1); }
        else if !e.is_dir && e.size > 0 && e.recoverable { let d = v.read(&e.node, 0, 16).unwrap(); println!("{}  head={:02x?}", "  ".repeat(depth), &d[..d.len().min(8)]); }
    }
}
fn main() {
    let p = std::env::args().nth(1).unwrap();
    let src = open_image(std::path::Path::new(&p)).unwrap();
    println!("acq: {:?}", src.acquisition());
    for part in volumes(src.as_ref()) {
        println!("== part {} {:?}", part.index, part.fs);
        let v = Volume::open(src.as_ref(), &part).unwrap();
        println!("{:?}", v.info());
        walk(&v, "root", 0);
    }
}
