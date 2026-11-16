//! Project chores, written in Rust so the repo needs no other tools.
//!
//!   cargo xtask package          release in target/package/dist: the setup
//!                                (with the app inside), a portable zip and
//!                                SHA256SUMS.txt. The release workflows run it.
//!                                On macOS: SteamInRust.app in a .dmg and
//!                                SHA256SUMS-macos.txt.
//!   cargo xtask steamclient32    the 32-bit steamclient DLL for 32-bit games,
//!     [--release]                where a debug/release build of the app finds it.

use sha2::{Digest, Sha256};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;

type Result<T = ()> = std::result::Result<T, String>;

fn main() {
    let task = std::env::args().nth(1).unwrap_or_default();
    let result = match task.as_str() {
        "package" => package(),
        "steamclient32" => steamclient32(std::env::args().any(|a| a == "--release")).map(|dll| println!("Built {}", dll.display())),
        _ => Err("usage: cargo xtask package | steamclient32 [--release]".into()),
    };
    if let Err(e) = result {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap().to_path_buf()
}

fn version(root: &Path) -> Result<String> {
    let toml = fs::read_to_string(root.join("Cargo.toml")).map_err(|e| e.to_string())?;
    toml.lines()
        .find_map(|l| l.strip_prefix("version = \"").and_then(|v| v.split('"').next()))
        .map(str::to_owned)
        .ok_or_else(|| "no version in Cargo.toml".into())
}

fn cargo(root: &Path, args: &[&str], env: &[(&str, &std::ffi::OsStr)], path_front: Option<&Path>) -> Result {
    let mut cmd = Command::new(std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into()));
    cmd.current_dir(root).args(args);
    for (k, v) in env {
        cmd.env(k, v);
    }
    if let Some(dir) = path_front {
        let mut paths = vec![dir.to_path_buf()];
        paths.extend(std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default()));
        cmd.env("PATH", std::env::join_paths(paths).map_err(|e| e.to_string())?);
    }
    let status = cmd.status().map_err(|e| format!("cargo: {e}"))?;
    if status.success() { Ok(()) } else { Err(format!("cargo {} failed", args.join(" "))) }
}

fn copy(from: &Path, to: &Path) -> Result {
    fs::copy(from, to).map(|_| ()).map_err(|e| format!("copy {}: {e}", from.display()))
}

/// A 32-bit MinGW gcc: SIR_MINGW32 (its bin folder), i686-w64-mingw32-gcc
/// on PATH, or the usual install folders (WinLibs in C:\tools, MSYS2).
fn gcc32() -> Option<PathBuf> {
    let exe = "i686-w64-mingw32-gcc.exe";
    if let Some(dir) = std::env::var_os("SIR_MINGW32") {
        return Some(Path::new(&dir).join(exe)).filter(|p| p.exists());
    }
    let usual = [r"C:\tools\winlibs32\mingw32\bin", r"C:\msys64\mingw32\bin", r"C:\mingw32\bin"].map(PathBuf::from);
    std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default()).chain(usual).map(|d| d.join(exe)).find(|p| p.exists())
}

/// Build the steamclient DLL for 32-bit games with a 32-bit MinGW. The app
/// finds it in target/i686-pc-windows-gnu/<profile>/ while developing.
fn steamclient32(release: bool) -> Result<PathBuf> {
    let root = root();
    let gcc = gcc32().ok_or("no 32-bit MinGW found: install one (e.g. WinLibs i686) and set SIR_MINGW32 to its bin folder")?;
    let status = Command::new("rustup").args(["target", "add", "i686-pc-windows-gnu"]).status();
    if !status.is_ok_and(|s| s.success()) {
        return Err("rustup target add i686-pc-windows-gnu failed".into());
    }
    let mut args = vec!["build", "-p", "steamclient", "--target", "i686-pc-windows-gnu"];
    if release {
        args.push("--release");
    }
    cargo(&root, &args, &[("CARGO_TARGET_I686_PC_WINDOWS_GNU_LINKER", gcc.as_os_str())], gcc.parent())?;
    Ok(root.join("target").join("i686-pc-windows-gnu").join(if release { "release" } else { "debug" }).join("steamclient.dll"))
}

fn package() -> Result {
    #[cfg(target_os = "macos")]
    return package_macos();
    #[cfg(not(target_os = "macos"))]
    package_windows()
}

/// Windows: the setup with the app inside, a portable zip and checksums.
#[cfg(not(target_os = "macos"))]
fn package_windows() -> Result {
    let root = root();
    let version = version(&root)?;
    println!("Packaging SteamInRust {version}");

    let pkg = root.join("target").join("package");
    let stage = pkg.join("stage");
    let dist = pkg.join("dist");
    let _ = fs::remove_dir_all(&pkg);
    for dir in [&stage, &dist] {
        fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }

    cargo(&root, &["build", "--release", "-p", "steam-rs", "-p", "steamclient"], &[], None)?;
    let release = root.join("target").join("release");
    copy(&release.join("steam-rs.exe"), &stage.join("steam-rs.exe"))?;
    copy(&release.join("steamclient.dll"), &stage.join("steamclient.dll"))?;

    // The same DLL for 32-bit games, when a 32-bit MinGW is around.
    if gcc32().is_some() {
        copy(&steamclient32(true)?, &stage.join("steamclient32.dll"))?;
    } else {
        eprintln!("warning: no 32-bit MinGW found: 32-bit games will start without Steam features (no steamclient32.dll).");
    }
    copy(&root.join("LICENSE"), &stage.join("LICENSE"))?;
    copy(&root.join("README.md"), &stage.join("README.md"))?;

    let payload = pkg.join("payload.zip");
    zip_dir(&stage, &payload)?;

    cargo(&root, &["build", "--release", "-p", "installer"], &[("SIR_PAYLOAD", payload.as_os_str())], None)?;
    copy(&release.join("SteamInRust-Setup.exe"), &dist.join(format!("SteamInRust-Setup-{version}.exe")))?;
    copy(&payload, &dist.join(format!("SteamInRust-{version}-portable.zip")))?;

    // LF line endings and no BOM, like sha256sum writes.
    let mut files: Vec<_> = fs::read_dir(&dist).map_err(|e| e.to_string())?.flatten().map(|e| e.path()).collect();
    files.sort();
    let mut sums = String::new();
    for f in &files {
        let bytes = fs::read(f).map_err(|e| e.to_string())?;
        let hash: String = Sha256::digest(&bytes).iter().map(|b| format!("{b:02x}")).collect();
        let name = f.file_name().unwrap().to_string_lossy();
        sums.push_str(&format!("{hash}  {name}\n"));
        println!("{name:<44} {:>7.1} MB", bytes.len() as f64 / 1_048_576.0);
    }
    fs::write(dist.join("SHA256SUMS.txt"), sums).map_err(|e| e.to_string())?;
    println!("Done: {}", dist.display());
    Ok(())
}

/// macOS: `SteamInRust.app` (unsigned, ad-hoc code signature) inside a
/// disk image with a link to Applications, plus its checksum.
#[cfg(target_os = "macos")]
fn package_macos() -> Result {
    let root = root();
    let version = version(&root)?;
    println!("Packaging SteamInRust {version} for macOS");
    let pkg = root.join("target").join("package");
    let stage = pkg.join("stage");
    let dist = pkg.join("dist");
    let _ = fs::remove_dir_all(&pkg);
    for dir in [&stage, &dist] {
        fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    cargo(&root, &["build", "--release", "-p", "steam-rs"], &[], None)?;

    let app = stage.join("SteamInRust.app");
    let (macos, resources) = (app.join("Contents").join("MacOS"), app.join("Contents").join("Resources"));
    for dir in [&macos, &resources] {
        fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    copy(&root.join("target").join("release").join("steam-rs"), &macos.join("steam-rs"))?;
    fs::write(app.join("Contents").join("Info.plist"), info_plist(&version)).map_err(|e| e.to_string())?;
    let png = fs::read(root.join("assets").join("icon-256.png")).map_err(|e| e.to_string())?;
    fs::write(resources.join("AppIcon.icns"), icns(&png)).map_err(|e| e.to_string())?;
    copy(&root.join("LICENSE"), &resources.join("LICENSE"))?;
    // Apple Silicon runs only signed programs: an ad-hoc signature (no
    // developer account) is enough for that; Gatekeeper still asks once.
    run(Command::new("codesign").args(["--force", "--deep", "--sign", "-"]).arg(&app))?;
    std::os::unix::fs::symlink("/Applications", stage.join("Applications")).map_err(|e| e.to_string())?;

    let dmg = dist.join(format!("SteamInRust-{version}-macos.dmg"));
    run(Command::new("hdiutil").args(["create", "-volname", "SteamInRust", "-fs", "HFS+", "-format", "UDZO", "-ov", "-srcfolder"]).arg(&stage).arg(&dmg))?;

    let bytes = fs::read(&dmg).map_err(|e| e.to_string())?;
    let hash: String = Sha256::digest(&bytes).iter().map(|b| format!("{b:02x}")).collect();
    let name = dmg.file_name().unwrap().to_string_lossy();
    fs::write(dist.join("SHA256SUMS-macos.txt"), format!("{hash}  {name}\n")).map_err(|e| e.to_string())?;
    println!("{name:<44} {:>7.1} MB", bytes.len() as f64 / 1_048_576.0);
    println!("Done: {}", dist.display());
    Ok(())
}

#[cfg(target_os = "macos")]
fn run(cmd: &mut Command) -> Result {
    let status = cmd.status().map_err(|e| format!("{cmd:?}: {e}"))?;
    if status.success() { Ok(()) } else { Err(format!("{cmd:?} failed")) }
}

#[cfg(target_os = "macos")]
fn info_plist(version: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
	<key>CFBundleName</key>
	<string>SteamInRust</string>
	<key>CFBundleDisplayName</key>
	<string>SteamInRust</string>
	<key>CFBundleIdentifier</key>
	<string>io.github.madonchik123.steaminrust</string>
	<key>CFBundleExecutable</key>
	<string>steam-rs</string>
	<key>CFBundleIconFile</key>
	<string>AppIcon</string>
	<key>CFBundlePackageType</key>
	<string>APPL</string>
	<key>CFBundleShortVersionString</key>
	<string>{version}</string>
	<key>CFBundleVersion</key>
	<string>{version}</string>
	<key>LSMinimumSystemVersion</key>
	<string>11.0</string>
	<key>LSApplicationCategoryType</key>
	<string>public.app-category.games</string>
	<key>NSHighResolutionCapable</key>
	<true/>
	<key>NSHumanReadableCopyright</key>
	<string>MIT License. Not affiliated with Valve.</string>
</dict>
</plist>
"#
    )
}

/// An .icns file holding one 256×256 PNG (type `ic08`), which macOS scales.
#[cfg(target_os = "macos")]
fn icns(png: &[u8]) -> Vec<u8> {
    let entry_len = 8 + png.len() as u32;
    let mut v = b"icns".to_vec();
    v.extend_from_slice(&(8 + entry_len).to_be_bytes());
    v.extend_from_slice(b"ic08");
    v.extend_from_slice(&entry_len.to_be_bytes());
    v.extend_from_slice(png);
    v
}

#[cfg(not(target_os = "macos"))]
/// Zip every file directly inside `dir` (the stage folder is flat).
fn zip_dir(dir: &Path, out: &Path) -> Result {
    let file = fs::File::create(out).map_err(|e| e.to_string())?;
    let mut zip = zip::ZipWriter::new(file);
    let options = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated).compression_level(Some(9));
    let mut entries: Vec<_> = fs::read_dir(dir).map_err(|e| e.to_string())?.flatten().map(|e| e.path()).filter(|p| p.is_file()).collect();
    entries.sort();
    for path in entries {
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        zip.start_file(name, options).map_err(|e| e.to_string())?;
        zip.write_all(&fs::read(&path).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    }
    zip.finish().map_err(|e| e.to_string())?;
    Ok(())
}
