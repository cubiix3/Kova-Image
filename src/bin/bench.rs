use kova_image::{
    decoder::{self, Target},
    security::Generation,
};
fn main() {
    let mut target = Target::full();
    let mut paths = Vec::new();
    let mut args = std::env::args_os().skip(1);
    while let Some(arg) = args.next() {
        if arg == "--fit" {
            let size = args.next().and_then(|v| v.into_string().ok());
            let parsed = size
                .as_deref()
                .and_then(|s| s.split_once('x'))
                .and_then(|(w, h)| Some((w.parse::<u32>().ok()?, h.parse::<u32>().ok()?)));
            let Some((max_width, max_height)) = parsed else {
                eprintln!("--fit expects WIDTHxHEIGHT, for example --fit 1920x1080");
                std::process::exit(2);
            };
            target = Target {
                max_width,
                max_height,
            };
        } else {
            paths.push(arg);
        }
    }
    if paths.is_empty() {
        eprintln!(
            "Usage: kova-bench [--fit WIDTHxHEIGHT] <image> [image ...]\n\
             Reports actual decode wall time; run a release build. --fit decodes\n\
             at the size a window of that many pixels needs, like the viewer does."
        );
        std::process::exit(2);
    }
    println!("format,width,height,frames,decoded_bytes,decode_ms");
    for path in paths {
        let start = std::time::Instant::now();
        let ticket = Generation::default().next();
        match decoder::load_target(std::path::Path::new(&path), &ticket, target, &mut |_| {}) {
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
