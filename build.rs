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
    println!("cargo:rerun-if-changed=assets/icon.ico");
    println!("cargo:rerun-if-changed=assets/BriskMarkdown.manifest");
    println!("cargo:rerun-if-changed=build.rs");
}
