//! Embeds the application icon and version metadata into `lanchat.exe`.
//!
//! Uses `winres`, which generates a `.rc` file from the Cargo package metadata
//! and compiles it with the Windows SDK resource compiler (`rc.exe`). If the
//! SDK cannot be located (e.g. a machine without the Windows SDK installed),
//! the resource step is skipped with a warning instead of failing the build —
//! the binary is fully functional without embedded resources.

fn main() {
    #[cfg(target_os = "windows")]
    {
        println!("cargo:rerun-if-changed=build.rs");
        println!("cargo:rerun-if-changed=assets/icon.ico");

        let mut res = winres::WindowsResource::new();
        res.set("ProductName", "LanChat");
        res.set(
            "FileDescription",
            "LanChat - open-source LAN messenger (FeiQ / IP Messenger compatible)",
        );
        res.set("LegalCopyright", "Copyright (c) LanChat contributors, MIT");
        res.set_icon("assets/icon.ico");
        if let Err(e) = res.compile() {
            println!(
                "cargo:warning=Skipping Windows resource embedding (rc.exe not found?): {}",
                e
            );
        }
    }
}
