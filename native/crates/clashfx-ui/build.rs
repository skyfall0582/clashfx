fn main() {
    println!("cargo:rerun-if-changed=../../assets/clashfx.ico");
    println!("cargo:rerun-if-changed=../../assets/clashfx-tray-off.ico");
    println!("cargo:rerun-if-changed=../../assets/clashfx-tray-on.ico");
    println!("cargo:rerun-if-changed=../../assets/clashfx-tray-busy.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        winresource::WindowsResource::new()
            .set_icon("../../assets/clashfx.ico")
            .set_icon_with_id("../../assets/clashfx-tray-off.ico", "2")
            .set_icon_with_id("../../assets/clashfx-tray-on.ico", "3")
            .set_icon_with_id("../../assets/clashfx-tray-busy.ico", "4")
            .set("ProductName", "ClashFx")
            .set("FileDescription", "ClashFx native proxy client")
            .set("OriginalFilename", "ClashFx.exe")
            .compile()
            .expect("embed ClashFx application icon");
    }
}
