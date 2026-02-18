use std::process::Command;
use std::time::SystemTime;

fn main() {
    let now = SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis();
    println!("cargo:rustc-env=BUILD_TIME={}", now);

    let branch = Command::new("git")
        .args(["rev-parse", "--abbrev-ref", "HEAD"])
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .unwrap_or_else(|| "unknown".to_string())
        .trim()
        .to_string();
    println!("cargo:rustc-env=GIT_BRANCH={}", branch);

    let commit = Command::new("git")
        .args(["rev-parse", "HEAD"])
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .unwrap_or_else(|| "unknown".to_string())
        .trim()
        .to_string();
    println!("cargo:rustc-env=GIT_COMMIT={}", commit);

    let commit_time = Command::new("git")
        .args(["show", "-s", "--format=%ct", "HEAD"])
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .and_then(|s| s.trim().parse::<u64>().ok())
        .map(|t| t * 1000)
        .unwrap_or(0);
    println!("cargo:rustc-env=GIT_COMMIT_TIME={}", commit_time);
}
