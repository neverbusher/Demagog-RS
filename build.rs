fn main() {
    println!("cargo:rerun-if-changed=assets/icon.ico");

    #[cfg(windows)]
    {
        let mut resources = tauri_winres::WindowsResource::new();
        resources.set_icon("assets/icon.ico");
        resources.set("ProductName", "Demagog-RS");
        resources.set(
            "FileDescription",
            "Text and dictionary processor for TTS",
        );
        resources.set("LegalCopyright", "MIT License");
        if let Err(error) = resources.compile() {
            panic!("failed to embed Windows resources and icon: {error}");
        }
    }
}
