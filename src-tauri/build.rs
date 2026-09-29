fn main() {
    if let Ok(url) = std::env::var("JOKERDECK_UPDATE_MANIFEST_URL") {
        println!("cargo:rustc-env=JOKERDECK_UPDATE_MANIFEST_URL={url}");
    }
    tauri_build::build()
}
