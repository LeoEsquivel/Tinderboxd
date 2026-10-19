//! Incrusta el icono y los datos de versión en los `.exe` de Windows.

fn main() {
    println!("cargo:rerun-if-changed=assets/icon.ico");
    println!("cargo:rerun-if-changed=build.rs");

    #[cfg(windows)]
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("assets/icon.ico")
            .set("ProductName", "Tinderboxd")
            .set("FileDescription", "Tinderboxd: tu Letterboxd de matches y citas")
            .set("LegalCopyright", "Tinderboxd");
        // Sin el SDK de Windows (rc.exe) se compila igual, sólo que sin icono.
        if let Err(e) = res.compile() {
            println!("cargo:warning=no se pudo incrustar el icono en el .exe: {e}");
        }
    }
}
