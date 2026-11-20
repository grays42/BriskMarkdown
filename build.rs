fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut res = winresource::WindowsResource::new();
        res.set_icon_with_id("assets/icon.ico", "1");
        res.set_manifest_file("assets/BriskMarkdown.manifest");
        res.set("FileDescription", "BriskMarkdown");
        res.set("ProductName", "BriskMarkdown");
        res.set("LegalCopyright", "MIT License");
        if let Err(e) = res.compile() {
            println!("cargo:warning=resources not embedded: {e}");
        }
    }
    // Load DLLs that are only needed for dialogs, printing, the clipboard's COM side or the
    // registry when they are first used, not at startup.
    if std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc") {
        for dll in ["shell32.dll", "comdlg32.dll", "ole32.dll", "oleaut32.dll", "imm32.dll", "advapi32.dll"] {
            println!("cargo:rustc-link-arg-bins=/DELAYLOAD:{dll}");
        }
        println!("cargo:rustc-link-lib=delayimp");
    }
    println!("cargo:rerun-if-changed=assets/icon.ico");
    println!("cargo:rerun-if-changed=assets/BriskMarkdown.manifest");
    println!("cargo:rerun-if-changed=build.rs");
}
