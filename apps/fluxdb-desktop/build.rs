fn main() {
    if std::env::var_os("CARGO_CFG_TARGET_OS").as_deref() == Some(std::ffi::OsStr::new("windows")) {
        println!("cargo:rerun-if-changed=assets/app-icon.ico");
        println!("cargo:rerun-if-changed=assets/windows.rc");
        embed_resource::compile("assets/windows.rc", embed_resource::NONE)
            .manifest_required()
            .expect("failed to embed Windows application icon");
    }
}
