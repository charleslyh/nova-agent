#[cfg(feature = "zeroclaw-sidecar")]
use std::env;
#[cfg(feature = "zeroclaw-sidecar")]
use std::fs;
#[cfg(feature = "zeroclaw-sidecar")]
use std::io::{BufRead, BufReader};
#[cfg(feature = "zeroclaw-sidecar")]
use std::path::PathBuf;
#[cfg(feature = "zeroclaw-sidecar")]
use std::process::{Command, Stdio};

fn main() {
    #[cfg(feature = "zeroclaw-sidecar")]
    build_zeroclaw_sidecar();

    tauri_build::build()
}

#[cfg(feature = "zeroclaw-sidecar")]
fn build_zeroclaw_sidecar() {
    // Rerun if zeroclaw source changes
    println!("cargo:rerun-if-changed=zeroclaw/src");
    println!("cargo:rerun-if-changed=zeroclaw/Cargo.toml");

    let profile = env::var("PROFILE").unwrap_or_else(|_| "debug".to_string());

    // Detect the target triple — use TARGET env (set by Cargo for cross-compilation),
    // falling back to the host triple.
    let target_triple = env::var("TARGET").unwrap_or_else(|_| {
        let output = Command::new("rustc")
            .args(["-vV"])
            .output()
            .expect("Failed to run rustc -vV");
        let stdout = String::from_utf8(output.stdout).unwrap();
        stdout
            .lines()
            .find(|l| l.starts_with("host:"))
            .map(|l| l.trim_start_matches("host:").trim().to_string())
            .expect("Could not determine host triple from rustc -vV")
    });

    // Use a separate target directory to avoid file lock deadlock.
    // The outer Cargo (building moray-app) holds a lock on `target/`,
    // so spawning another `cargo build` against the same `target/` will deadlock.
    let workspace_root = PathBuf::from("../");
    let sidecar_target_dir = workspace_root.join("target-sidecar");

    eprintln!("[zeroclaw-sidecar] Building zeroclaw CLI for target: {target_triple}");
    eprintln!(
        "[zeroclaw-sidecar] Using separate target dir: {}",
        sidecar_target_dir.display()
    );

    let sidecar_target_str = sidecar_target_dir.to_string_lossy().to_string();

    // Build zeroclaw CLI for the same target, using isolated target dir
    let mut args = vec![
        "build",
        "--manifest-path",
        "../zeroclaw/Cargo.toml",
        "--target-dir",
        &sidecar_target_str,
        "--target",
        &target_triple,
    ];
    if profile == "release" {
        args.push("--release");
    }

    let mut child = Command::new("cargo")
        .args(&args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("Failed to spawn ZeroClaw CLI build");

    // Stream stderr in real-time so user can see compilation progress
    if let Some(stderr) = child.stderr.take() {
        let reader = BufReader::new(stderr);
        for line in reader.lines() {
            if let Ok(line) = line {
                eprintln!("[zeroclaw-sidecar] {line}");
            }
        }
    }

    let status = child.wait().expect("Failed to wait for ZeroClaw CLI build");
    if !status.success() {
        panic!("Failed to build ZeroClaw CLI for target {target_triple}");
    }

    eprintln!("[zeroclaw-sidecar] Build completed successfully.");

    // Source binary path: target-sidecar/<target>/<profile>/zeroclaw
    let profile_dir = if profile == "release" {
        "release"
    } else {
        "debug"
    };
    let src_binary = sidecar_target_dir.join(format!("{target_triple}/{profile_dir}/zeroclaw"));

    if !src_binary.exists() {
        panic!("ZeroClaw binary not found at {}", src_binary.display());
    }

    // Destination: binaries/zeroclaw-<target_triple>
    // Tauri sidecar naming convention: the binary name must match
    // the externalBin entry + target triple suffix.
    let binaries_dir = PathBuf::from("binaries");
    fs::create_dir_all(&binaries_dir).expect("Failed to create binaries/ directory");

    let dest_binary = binaries_dir.join(format!("zeroclaw-{target_triple}"));
    fs::copy(&src_binary, &dest_binary).unwrap_or_else(|e| {
        panic!(
            "Failed to copy {} -> {}: {e}",
            src_binary.display(),
            dest_binary.display()
        )
    });

    println!(
        "cargo:warning=Copied zeroclaw sidecar to {}",
        dest_binary.display()
    );
}
