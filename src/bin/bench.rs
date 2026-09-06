use kova_image::{decoder, security::Generation};
fn main() {
    let paths: Vec<_> = std::env::args_os().skip(1).collect();
    if paths.is_empty() {
        eprintln!(
            "Usage: kova-bench <image> [image ...]\nReports actual decode wall time; run a release build."
        );
        std::process::exit(2);
    }
    println!("format,width,height,frames,decoded_bytes,decode_ms");
    for path in paths {
        let start = std::time::Instant::now();
        match decoder::load(std::path::Path::new(&path), &Generation::default().next()) {
            Ok(image) => println!(
                "{:?},{},{},{},{},{:.3}",
                image.format,
                image.width,
                image.height,
                image.frames.len(),
                image.weight(),
                start.elapsed().as_secs_f64() * 1000.
            ),
            Err(e) => {
                eprintln!("{}: {e}", path.to_string_lossy());
                std::process::exit(1);
            }
        }
    }
}
