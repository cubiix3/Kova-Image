fn main() {
    slint_build::compile("ui/main.slint").expect("compile viewer UI");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        println!("cargo:rerun-if-changed=assets/kova.ico");
        println!("cargo:rerun-if-changed=assets/app.manifest");
        winresource::WindowsResource::new()
            .set_icon("assets/kova.ico")
            .set_manifest_file("assets/app.manifest")
            .set("ProductName", "Kova Image")
            .set("FileDescription", "Kova Image Viewer")
            .set("OriginalFilename", "kova-image.exe")
            .set("LegalCopyright", "Copyright 2026 Kova Contributors")
            .compile()
            .expect("compile Windows resources");
    }
}
