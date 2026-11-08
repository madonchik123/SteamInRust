//! Embeds the app (a zip made by `cargo xtask package`, passed as SIR_PAYLOAD)
//! and the icon. Without SIR_PAYLOAD the setup builds with an empty payload
//! and says so when run (handy for working on its look).

use std::path::PathBuf;

fn main() {
    println!("cargo:rerun-if-env-changed=SIR_PAYLOAD");
    let out = PathBuf::from(std::env::var("OUT_DIR").unwrap()).join("payload.zip");
    match std::env::var_os("SIR_PAYLOAD") {
        Some(src) => {
            println!("cargo:rerun-if-changed={}", PathBuf::from(&src).display());
            std::fs::copy(&src, &out).expect("SIR_PAYLOAD must point to the app zip");
        }
        None => std::fs::write(&out, []).unwrap(),
    }
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("../../assets/icon.ico")
            .set("ProductName", "SteamInRust")
            .set("FileDescription", "SteamInRust Setup")
            .set("CompanyName", "SteamInRust")
            .set("LegalCopyright", "MIT License. Not affiliated with Valve.");
        // Per-user install: never ask for administrator rights.
        res.set_manifest(
            r#"<assembly xmlns="urn:schemas-microsoft-com:asm.v1" manifestVersion="1.0">
<trustInfo xmlns="urn:schemas-microsoft-com:asm.v3"><security><requestedPrivileges>
<requestedExecutionLevel level="asInvoker" uiAccess="false"/>
</requestedPrivileges></security></trustInfo>
<application xmlns="urn:schemas-microsoft-com:asm.v3"><windowsSettings>
<dpiAware xmlns="http://schemas.microsoft.com/SMI/2005/WindowsSettings">true/pm</dpiAware>
<dpiAwareness xmlns="http://schemas.microsoft.com/SMI/2016/WindowsSettings">PerMonitorV2</dpiAwareness>
</windowsSettings></application>
</assembly>"#,
        );
        if let Err(e) = res.compile() {
            println!("cargo:warning=icon not embedded: {e}");
        }
    }
}
