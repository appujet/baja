// Build script: embeds git branch, commit SHA, and build timestamp into the
// binary via CARGO_PKG_* properties (and custom ENV vars) at compile time.
//
// Strategy:
//   1. Check for CI environment variables (e.g., GITHUB_SHA, GITHUB_REF_NAME).
//   2. Try `git` commands (e.g., `git rev-parse`, `git log`) via CLI.
//   3. Fall back to reading `.git/HEAD` directly.

use std::{env, fs, path::Path, process::Command, time::SystemTime};

fn main() {
    let now = SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis();
    println!("cargo:rustc-env=BUILD_TIME={}", now);

    // Tell Cargo to rerun this script if git state changes
    println!("cargo:rerun-if-changed=.git/HEAD");
    if Path::new(".git/refs/heads").exists() {
        println!("cargo:rerun-if-changed=.git/refs/heads");
    }

    let git_info = get_git_info();

    println!("cargo:rustc-env=GIT_BRANCH={}", git_info.branch);
    println!("cargo:rustc-env=GIT_COMMIT={}", git_info.commit);
    println!("cargo:rustc-env=GIT_COMMIT_TIME={}", git_info.commit_time);
}

struct GitInfo {
    branch: String,
    commit: String,
    commit_time: u64,
}

fn get_git_info() -> GitInfo {
    let mut info = GitInfo {
        branch: "unknown".to_string(),
        commit: "unknown".to_string(),
        commit_time: 0,
    };

    // 1. Try CI environment variables first (perfect for GitHub Actions)
    if let Ok(gh_ref) = env::var("GITHUB_REF_NAME") {
        info.branch = gh_ref;
    }
    if let Ok(gh_sha) = env::var("GITHUB_SHA") {
        info.commit = gh_sha;
    }

    // 2. Try git command
    if info.branch == "unknown" {
        if let Ok(output) = Command::new("git")
            .args(["rev-parse", "--abbrev-ref", "HEAD"])
            .output()
        {
            if output.status.success() {
                info.branch = String::from_utf8_lossy(&output.stdout).trim().to_string();
            }
        }
    }

    if info.commit == "unknown" {
        if let Ok(output) = Command::new("git").args(["rev-parse", "HEAD"]).output() {
            if output.status.success() {
                info.commit = String::from_utf8_lossy(&output.stdout).trim().to_string();
            }
        }
    }

    if let Ok(output) = Command::new("git")
        .args(["show", "-s", "--format=%ct", "HEAD"])
        .output()
    {
        if output.status.success() {
            if let Ok(s) = String::from_utf8(output.stdout) {
                if let Ok(t) = s.trim().parse::<u64>() {
                    info.commit_time = t * 1000;
                }
            }
        }
    }

    // 3. Fallback to manual parsing if we still don't have git CLI or CI envs
    if info.commit == "unknown" || info.branch == "unknown" {
        if let Ok(head) = fs::read_to_string(".git/HEAD") {
            if head.starts_with("ref: ") {
                let ref_path = head.trim_start_matches("ref: ").trim();
                info.branch = ref_path.split('/').last().unwrap_or("unknown").to_string();

                let full_ref_path = format!(".git/{}", ref_path);
                if let Ok(commit) = fs::read_to_string(full_ref_path) {
                    info.commit = commit.trim().to_string();
                }
            } else {
                info.commit = head.trim().to_string();
            }
        }
    }

    // Try to get commit time from file metadata if git failed
    if info.commit_time == 0 && info.commit != "unknown" {
        let full_ref_path = format!(".git/refs/heads/{}", info.branch);
        if let Ok(metadata) = fs::metadata(full_ref_path) {
            if let Ok(modified) = metadata.modified() {
                if let Ok(duration) = modified.duration_since(SystemTime::UNIX_EPOCH) {
                    info.commit_time = duration.as_millis() as u64;
                }
            }
        }
    }

    info
}
