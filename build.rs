use std::{
    env, fs,
    path::Path,
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};

fn main() {
    println!("cargo:rerun-if-changed=.git/HEAD");
    if Path::new(".git/refs/heads").exists() {
        println!("cargo:rerun-if-changed=.git/refs/heads");
    }
    println!("cargo:rerun-if-env-changed=GITHUB_SHA");
    println!("cargo:rerun-if-env-changed=GITHUB_REF_NAME");

    let now_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock before Unix epoch")
        .as_millis();
    println!("cargo:rustc-env=BUILD_TIME={}", now_ms);
    println!(
        "cargo:rustc-env=BUILD_TIME_HUMAN={}",
        format_timestamp_ms(now_ms as u64)
    );

    let rust_version = Command::new("rustc")
        .arg("--version")
        .output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_owned())
        .unwrap_or_else(|| {
            env::var("RUSTC").unwrap_or_else(|_| "unknown".into())
        });
    println!("cargo:rustc-env=RUST_VERSION={}", rust_version);

    let git = gather_git_info();

    println!("cargo:rustc-env=GIT_BRANCH={}", git.branch);
    println!("cargo:rustc-env=GIT_COMMIT={}", git.commit);
    println!("cargo:rustc-env=GIT_COMMIT_SHORT={}", git.commit_short);
    println!("cargo:rustc-env=GIT_COMMIT_TIME={}", git.commit_time_ms);
    println!(
        "cargo:rustc-env=GIT_COMMIT_TIME_HUMAN={}",
        format_timestamp_ms(git.commit_time_ms)
    );
    println!(
        "cargo:rustc-env=GIT_DIRTY={}",
        if git.dirty { "true" } else { "false" }
    );

    let version_string = build_version_string(&git);
    println!("cargo:rustc-env=GIT_VERSION_STRING={}", version_string);
}


struct GitInfo {
    branch: String,
    /// Full commit SHA (40 hex chars when available)
    commit: String,
    /// Short commit SHA (7 hex chars when available)
    commit_short: String,
    /// Unix timestamp of the commit in **milliseconds**
    commit_time_ms: u64,
    /// Whether the working tree has uncommitted changes
    dirty: bool,
}

impl Default for GitInfo {
    fn default() -> Self {
        Self {
            branch: "unknown".into(),
            commit: "unknown".into(),
            commit_short: "unknown".into(),
            commit_time_ms: 0,
            dirty: false,
        }
    }
}


fn gather_git_info() -> GitInfo {
    let mut info = GitInfo::default();

    if let Ok(v) = env::var("GITHUB_REF_NAME") {
        if !v.is_empty() {
            info.branch = v;
        }
    }
    if let Ok(v) = env::var("GITHUB_SHA") {
        if !v.is_empty() {
            info.commit = v.clone();
            info.commit_short = v.chars().take(7).collect();
        }
    }

    if info.branch == "unknown" {
        info.branch = git_output(&["rev-parse", "--abbrev-ref", "HEAD"])
            .unwrap_or_else(|| "unknown".into());
    }

    if info.commit == "unknown" {
        if let Some(full) = git_output(&["rev-parse", "HEAD"]) {
            info.commit_short = full.chars().take(7).collect();
            info.commit = full;
        }
    } else if info.commit_short == "unknown" {
        info.commit_short = info.commit.chars().take(7).collect();
    }

    if let Some(ts) = git_output(&["show", "-s", "--format=%ct", "HEAD"])
        .and_then(|s| s.trim().parse::<u64>().ok())
    {
        info.commit_time_ms = ts * 1_000;
    }

    info.dirty = git_output(&["status", "--porcelain"])
        .map(|s| !s.trim().is_empty())
        .unwrap_or(false);

    if info.commit == "unknown" || info.branch == "unknown" {
        if let Some((branch, commit)) = parse_dot_git_head() {
            if info.branch == "unknown" {
                info.branch = branch;
            }
            if info.commit == "unknown" && !commit.is_empty() {
                info.commit_short = commit.chars().take(7).collect();
                info.commit = commit;
            }
        }
    }

    if info.commit_time_ms == 0 && info.branch != "unknown" {
        let ref_path = format!(".git/refs/heads/{}", info.branch);
        info.commit_time_ms = file_mtime_ms(&ref_path).unwrap_or(0);
    }

    info
}


/// Run a git command and return trimmed stdout, or `None` on failure.
fn git_output(args: &[&str]) -> Option<String> {
    let out = Command::new("git").args(args).output().ok()?;
    if out.status.success() {
        Some(String::from_utf8_lossy(&out.stdout).trim().to_owned())
    } else {
        None
    }
}

/// Parse `.git/HEAD` manually, returning `(branch, commit)`.
fn parse_dot_git_head() -> Option<(String, String)> {
    let head = fs::read_to_string(".git/HEAD").ok()?;
    let head = head.trim();

    if let Some(ref_path) = head.strip_prefix("ref: ") {
        // Symbolic ref — e.g. "ref: refs/heads/main"
        let branch = ref_path
            .split('/')
            .last()
            .unwrap_or("unknown")
            .to_owned();

        let commit = fs::read_to_string(format!(".git/{}", ref_path))
            .ok()
            .map(|s| s.trim().to_owned())
            // Also try packed-refs
            .or_else(|| packed_ref_lookup(ref_path))
            .unwrap_or_default();

        Some((branch, commit))
    } else {
        // Detached HEAD — the file itself is the SHA
        Some(("HEAD".into(), head.to_owned()))
    }
}

/// Look up a ref in `.git/packed-refs`.
fn packed_ref_lookup(ref_name: &str) -> Option<String> {
    let packed = fs::read_to_string(".git/packed-refs").ok()?;
    for line in packed.lines() {
        if line.starts_with('#') {
            continue;
        }
        let mut parts = line.splitn(2, ' ');
        if let (Some(sha), Some(name)) = (parts.next(), parts.next()) {
            if name.trim() == ref_name {
                return Some(sha.trim().to_owned());
            }
        }
    }
    None
}

/// Return the mtime of a file as Unix milliseconds.
fn file_mtime_ms(path: &str) -> Option<u64> {
    let meta = fs::metadata(path).ok()?;
    let mtime = meta.modified().ok()?;
    let ms = mtime
        .duration_since(UNIX_EPOCH)
        .ok()?
        .as_millis() as u64;
    Some(ms)
}

/// Build a human-readable version string, e.g. `main@a1b2c3d` or `main@a1b2c3d-dirty`.
fn build_version_string(git: &GitInfo) -> String {
    let dirty = if git.dirty { "-dirty" } else { "" };
    format!("{}@{}{}", git.branch, git.commit_short, dirty)
}

/// Format a Unix timestamp (milliseconds) as `DD.MM.YYYY HH:MM:SS UTC`.
/// Pure Rust, no external crates required.
fn format_timestamp_ms(ms: u64) -> String {
    if ms == 0 {
        return "unknown".into();
    }
    let secs = ms / 1_000;
    // Gregorian calendar decomposition (valid for dates 1970–2099)
    let days_since_epoch = secs / 86_400;
    let time_of_day = secs % 86_400;

    let hh = time_of_day / 3_600;
    let mm = (time_of_day % 3_600) / 60;
    let ss = time_of_day % 60;

    let (year, month, day) = days_to_ymd(days_since_epoch as u32);

    format!(
        "{:02}.{:02}.{} {:02}:{:02}:{:02} UTC",
        day, month, year, hh, mm, ss
    )
}

fn days_to_ymd(mut days: u32) -> (u32, u32, u32) {
    days += 719_468;

    let era = days / 146_097;
    let doe = days % 146_097; // day of era [0, 146096]
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365; // year of era
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // day of year [0, 365]
    let mp = (5 * doy + 2) / 153; // internal month
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };

    (y, m, d)
}