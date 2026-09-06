//! Manual native recycle check, using only a newly generated temporary file.
use kova_image::{decoder::Stamp, windows_integration as native};
fn main() -> Result<(), Box<dyn std::error::Error>> {
    if std::env::args().nth(1).as_deref() != Some("--generated-file-only") {
        return Err(
            "Run with --generated-file-only to test recycling an original temporary image".into(),
        );
    }
    let _apartment = native::Apartment::new()?;
    let folder =
        std::env::temp_dir().join(format!("kova-image-recycle-smoke-{}", std::process::id()));
    std::fs::create_dir(&folder)?;
    let path = folder.join("generated-猫.png");
    image::RgbaImage::from_pixel(2, 2, image::Rgba([120, 180, 240, 255])).save(&path)?;
    let stamp = Stamp::read(&path)?;
    native::recycle(0, &path, &stamp)?;
    if path.exists() {
        return Err("Shell returned success but the generated file still exists".into());
    }
    std::fs::remove_dir(&folder)?;
    println!("PASS: generated Unicode-path PNG moved to Windows Recycle Bin");
    Ok(())
}
