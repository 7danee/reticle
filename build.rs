fn main() {
    println!("cargo:rerun-if-changed=assets/icon/reticle.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows")
        && std::path::Path::new("assets/icon/reticle.ico").exists()
    {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("assets/icon/reticle.ico");
        res.set("ProductName", "reticle");
        res.set("FileDescription", "reticle – Crosshair-Overlay");
        if let Err(e) = res.compile() {
            println!("cargo:warning=icon resource not embedded: {e}");
        }
    }
}
