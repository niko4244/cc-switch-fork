fn main() {
    tauri_build::build();

    // Local fork identity: stamp the commit this binary was built from so the
    // app can name the exact fork build it is running (and so a stale artifact
    // is obvious in the UI). Falls back to "unknown" rather than failing a
    // build when git is absent.
    println!(
        "cargo:rustc-env=CC_SWITCH_FORK_COMMIT={}",
        fork_commit_stamp()
    );

    // Windows: Embed Common Controls v6 manifest for test binaries
    //
    // When running `cargo test`, the generated test executables don't include
    // the standard Tauri application manifest. Without Common Controls v6,
    // `tauri::test` calls fail with STATUS_ENTRYPOINT_NOT_FOUND.
    //
    // This workaround:
    // 1. Embeds the manifest into test binaries via /MANIFEST:EMBED
    // 2. Uses /MANIFEST:NO for the main binary to avoid duplicate resources
    //    (Tauri already handles manifest embedding for the app binary)
    #[cfg(target_os = "windows")]
    {
        let manifest_path = std::path::PathBuf::from(
            std::env::var("CARGO_MANIFEST_DIR").expect("missing CARGO_MANIFEST_DIR"),
        )
        .join("common-controls.manifest");
        let manifest_arg = format!("/MANIFESTINPUT:{}", manifest_path.display());

        println!("cargo:rustc-link-arg=/MANIFEST:EMBED");
        println!("cargo:rustc-link-arg={}", manifest_arg);
        // Avoid duplicate manifest resources in binary builds.
        println!("cargo:rustc-link-arg-bins=/MANIFEST:NO");
        println!("cargo:rerun-if-changed={}", manifest_path.display());
    }
}

/// Short HEAD plus a `-dirty` marker, or `unknown` outside a git checkout.
fn fork_commit_stamp() -> String {
    let manifest_dir =
        std::path::PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap_or_default());

    // Re-stamp when HEAD moves so the embedded commit cannot go stale, but only
    // for paths that exist: a printed `rerun-if-changed` for a missing file is
    // just noise.
    if let Some(repo) = manifest_dir.parent() {
        for candidate in [
            repo.join(".git").join("HEAD"),
            repo.join(".git").join("index"),
        ] {
            if candidate.exists() {
                println!("cargo:rerun-if-changed={}", candidate.display());
            }
        }
    }

    let Some(head) = run_git(&manifest_dir, &["rev-parse", "--short", "HEAD"]) else {
        return "unknown".to_string();
    };
    let dirty = run_git(&manifest_dir, &["status", "--porcelain"])
        .map(|status| !status.trim().is_empty())
        .unwrap_or(false);

    if dirty {
        format!("{head}-dirty")
    } else {
        head
    }
}

fn run_git(cwd: &std::path::Path, args: &[&str]) -> Option<String> {
    let output = std::process::Command::new("git")
        .args(args)
        .current_dir(cwd)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if stdout.is_empty() {
        None
    } else {
        Some(stdout)
    }
}
