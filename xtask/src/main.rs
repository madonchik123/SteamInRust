//! Project chores, written in Rust so the repo needs no other tools.
//!
//!   cargo xtask package
//!
//! Builds a release in target/package/dist: the setup (with the app inside),
//! a portable zip and SHA256SUMS.txt. The release workflows run this too.

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
        _ => Err("usage: cargo xtask package".into()),
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

/// A 32-bit MinGW gcc: SIR_MINGW32 (its bin folder) or i686-w64-mingw32-gcc on PATH.
fn gcc32() -> Option<PathBuf> {
    let exe = "i686-w64-mingw32-gcc.exe";
    if let Some(dir) = std::env::var_os("SIR_MINGW32") {
        return Some(Path::new(&dir).join(exe)).filter(|p| p.exists());
    }
    std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default()).map(|d| d.join(exe)).find(|p| p.exists())
}

fn package() -> Result {
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
    if let Some(gcc) = gcc32() {
        let status = Command::new("rustup").args(["target", "add", "i686-pc-windows-gnu"]).status();
        if !status.is_ok_and(|s| s.success()) {
            return Err("rustup target add i686-pc-windows-gnu failed".into());
        }
        cargo(
            &root,
            &["build", "--release", "-p", "steamclient", "--target", "i686-pc-windows-gnu"],
            &[("CARGO_TARGET_I686_PC_WINDOWS_GNU_LINKER", gcc.as_os_str())],
            gcc.parent(),
        )?;
        copy(&root.join("target/i686-pc-windows-gnu/release/steamclient.dll"), &stage.join("steamclient32.dll"))?;
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
