fn main() {
    slint_build::compile("ui/main.slint").expect("compile viewer UI");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        println!("cargo:rerun-if-env-changed=KOVA_FFMPEG_DIR");
        println!("cargo:rerun-if-changed=src/video/ffmpeg.c");
        let sdk = std::env::var_os("KOVA_FFMPEG_DIR")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| std::path::PathBuf::from("artifacts/ffmpeg/sdk"));
        assert!(
            sdk.join("include/libavcodec/avcodec.h").is_file(),
            "Run scripts/build-ffmpeg.ps1 before building Kova Image"
        );
        let sdk = std::env::current_dir()
            .expect("project directory")
            .join(sdk);
        for folder in ["include", "lib", "bin"] {
            println!("cargo:rerun-if-changed={}", sdk.join(folder).display());
        }
        cc::Build::new()
            .file("src/video/ffmpeg.c")
            .flag(format!("/external:I{}", sdk.join("include").display()))
            .flag("/external:W0")
            .warnings_into_errors(true)
            .compile("kova_video");
        println!(
            "cargo:rustc-link-search=native={}",
            sdk.join("lib").display()
        );
        println!(
            "cargo:rustc-link-search=native={}",
            sdk.join("bin").display()
        );
        for library in ["avformat", "avcodec", "avutil", "swscale", "swresample"] {
            println!("cargo:rustc-link-lib=dylib={library}");
        }
        // Cargo runs examples and tests from these directories as well.
        let out = std::path::PathBuf::from(std::env::var_os("OUT_DIR").unwrap());
        let profile = out.ancestors().nth(3).unwrap();
        for folder in [
            profile.to_path_buf(),
            profile.join("deps"),
            profile.join("examples"),
        ] {
            std::fs::create_dir_all(&folder).expect("create runtime directory");
            for dll in std::fs::read_dir(sdk.join("bin")).expect("FFmpeg DLLs") {
                let dll = dll.unwrap().path();
                if dll.extension().is_some_and(|e| e == "dll") {
                    std::fs::copy(&dll, folder.join(dll.file_name().unwrap()))
                        .expect("copy FFmpeg DLL");
                }
            }
        }
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
