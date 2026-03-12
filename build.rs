use std::{
    env, fs,
    path::Path,
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};

fn main() {
    setup_rerun_triggers();

    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;

    emit_env("BUILD_TIME", now);
    emit_env("BUILD_TIME_HUMAN", format_timestamp(now));
    emit_env("RUST_VERSION", get_rustc_version());

    let git = GitInfo::gather();
    git.emit();

    if let Some(pre) = detect_pre_release() {
        emit_env("RUSTALINK_PRE_RELEASE", pre);
    }
}

/// Configure cargo to rerun the build script if these files or variables change.
fn setup_rerun_triggers() {
    println!("cargo:rerun-if-changed=src");
    println!("cargo:rerun-if-changed=.git/HEAD");
    if Path::new(".git/refs/heads").exists() {
        println!("cargo:rerun-if-changed=.git/refs/heads");
    }
    if Path::new(".git/refs/tags").exists() {
        println!("cargo:rerun-if-changed=.git/refs/tags");
    }
    if Path::new(".git/packed-refs").exists() {
        println!("cargo:rerun-if-changed=.git/packed-refs");
    }
    println!("cargo:rerun-if-env-changed=GITHUB_SHA");
    println!("cargo:rerun-if-env-changed=GITHUB_REF_NAME");
    println!("cargo:rerun-if-env-changed=GITHUB_REF");
    println!("cargo:rerun-if-env-changed=RUSTUP_TOOLCHAIN");
    println!("cargo:rerun-if-env-changed=RUSTC");
}

fn emit_env<V: std::fmt::Display>(name: &str, value: V) {
    println!("cargo:rustc-env={}={}", name, value);
}

fn get_rustc_version() -> String {
    let rustc = env::var("RUSTC").unwrap_or_else(|_| "rustc".to_string());
    Command::new(rustc)
        .arg("--version")
        .output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_owned())
        .unwrap_or_else(|| "unknown".into())
}

#[derive(Debug, Default)]
struct GitInfo {
    branch: String,
    commit: String,
    commit_short: String,
    commit_time_ms: u64,
    dirty: bool,
}

impl GitInfo {
    fn gather() -> Self {
        let mut info = Self::default();

        // 1. Try environment variables (CI/CD)
        if let Ok(v) = env::var("GITHUB_REF_NAME") {
            info.branch = v;
        }
        if let Ok(v) = env::var("GITHUB_SHA") {
            info.commit = v.clone();
            info.commit_short = v.chars().take(7).collect();
        }

        // 2. Fetch from git command if still unknown
        if info.branch.is_empty() || info.branch == "unknown" {
            info.branch = git_output(&["rev-parse", "--abbrev-ref", "HEAD"])
                .unwrap_or_else(|| "unknown".into());
        }

        if info.commit.is_empty() || info.commit == "unknown" {
            if let Some(full) = git_output(&["rev-parse", "HEAD"]) {
                info.commit = full.clone();
                info.commit_short = full.chars().take(7).collect();
            } else {
                info.commit = "unknown".into();
                info.commit_short = "unknown".into();
            }
        }

        // 3. Metadata
        if let Some(ts) = git_output(&["show", "-s", "--format=%ct", "HEAD"])
            .and_then(|s| s.trim().parse::<u64>().ok())
        {
            info.commit_time_ms = ts * 1000;
        }

        info.dirty = git_output(&["status", "--porcelain"])
            .map(|s| !s.trim().is_empty())
            .unwrap_or(false);

        // 4. Fallback to manual file parsing if git command failed
        if (info.commit == "unknown" || info.branch == "unknown")
            && let Some((branch, commit)) = parse_dot_git_head()
        {
            if info.branch == "unknown" {
                info.branch = branch;
            }
            if info.commit == "unknown" && !commit.is_empty() {
                info.commit = commit.clone();
                info.commit_short = commit.chars().take(7).collect();
            }
        }

        if info.commit_time_ms == 0 && info.branch != "unknown" {
            let ref_path = format!(".git/refs/heads/{}", info.branch);
            info.commit_time_ms = file_mtime_ms(&ref_path).unwrap_or(0);
        }

        info
    }

    fn emit(&self) {
        emit_env("GIT_BRANCH", &self.branch);
        emit_env("GIT_COMMIT", &self.commit);
        emit_env("GIT_COMMIT_SHORT", &self.commit_short);
        emit_env("GIT_COMMIT_TIME", self.commit_time_ms);
        emit_env(
            "GIT_COMMIT_TIME_HUMAN",
            format_timestamp(self.commit_time_ms),
        );
        emit_env("GIT_DIRTY", self.dirty);

        let dirty_suffix = if self.dirty { "-dirty" } else { "" };
        emit_env(
            "GIT_VERSION_STRING",
            format!("{}@{}{}", self.branch, self.commit_short, dirty_suffix),
        );
    }
}

fn detect_pre_release() -> Option<String> {
    let is_tag = env::var("GITHUB_REF")
        .map(|r| r.starts_with("refs/tags/"))
        .unwrap_or(false);

    // Priority 1: GITHUB_REF_NAME (tag or branch)
    if let Ok(v) = env::var("GITHUB_REF_NAME") {
        if is_tag {
            if let Some(idx) = v.find('-')
                && let Some(sanitized) = sanitize_pre_release(&v[idx + 1..])
            {
                return Some(sanitized);
            }
        } else {
            // Use non-main branches as pre-release identifiers
            if !is_main_branch(&v)
                && let Some(sanitized) = sanitize_pre_release(&v)
            {
                return Some(sanitized);
            }
        }
    }

    // Priority 2: GITHUB_REF (standard tag format)
    if is_tag
        && let Ok(v) = env::var("GITHUB_REF")
        && let Some(idx) = v.rfind('-')
        && let Some(sanitized) = sanitize_pre_release(&v[idx + 1..])
    {
        return Some(sanitized);
    }

    // Priority 3: Git describe
    if let Some(desc) = git_output(&["describe", "--tags", "--always"])
        && let Some(idx) = desc.find('-')
    {
        let part = &desc[idx + 1..];
        // Handle cases like v1.0.8-beta.1-2-gabc123
        if let Some(next_dash) = part.find('-') {
            let pre = &part[..next_dash];
            if !is_numeric(pre)
                && let Some(sanitized) = sanitize_pre_release(pre)
            {
                return Some(sanitized);
            }
        } else if !is_numeric(part)
            && let Some(sanitized) = sanitize_pre_release(part)
        {
            return Some(sanitized);
        }
    }

    // Priority 4: Local branch name
    if let Some(branch) = git_output(&["rev-parse", "--abbrev-ref", "HEAD"])
        && !is_main_branch(&branch)
        && branch != "HEAD"
        && !branch.is_empty()
        && let Some(sanitized) = sanitize_pre_release(&branch)
    {
        return Some(sanitized);
    }

    None
}

fn sanitize_pre_release(s: &str) -> Option<String> {
    let mut result = String::with_capacity(s.len());
    let mut last_was_dash = false;

    for c in s.chars() {
        if c.is_ascii_alphanumeric() || c == '.' {
            result.push(c);
            last_was_dash = false;
        } else if (c == '/' || c == '-' || c == '_' || c.is_whitespace())
            && !last_was_dash
            && !result.is_empty()
        {
            result.push('-');
            last_was_dash = true;
        }
    }

    if result.ends_with('-') {
        result.pop();
    }

    let final_s = result.chars().take(40).collect::<String>();
    if final_s.is_empty() || is_numeric(&final_s) {
        None
    } else {
        Some(final_s)
    }
}

fn is_main_branch(name: &str) -> bool {
    matches!(name, "main" | "master")
}

fn is_numeric(s: &str) -> bool {
    !s.is_empty() && s.chars().all(char::is_numeric)
}

fn git_output(args: &[&str]) -> Option<String> {
    let out = Command::new("git").args(args).output().ok()?;
    if out.status.success() {
        Some(String::from_utf8_lossy(&out.stdout).trim().to_owned())
    } else {
        None
    }
}

fn parse_dot_git_head() -> Option<(String, String)> {
    let head = fs::read_to_string(".git/HEAD").ok()?.trim().to_owned();

    if let Some(ref_path) = head.strip_prefix("ref: ") {
        let branch = ref_path
            .strip_prefix("refs/heads/")
            .unwrap_or(ref_path)
            .to_owned();
        let commit = fs::read_to_string(format!(".git/{}", ref_path))
            .ok()
            .map(|s| s.trim().to_owned())
            .or_else(|| packed_ref_lookup(ref_path))
            .unwrap_or_default();
        Some((branch, commit))
    } else {
        Some(("HEAD".into(), head))
    }
}

fn packed_ref_lookup(ref_name: &str) -> Option<String> {
    let packed = fs::read_to_string(".git/packed-refs").ok()?;
    for line in packed.lines().filter(|l| !l.starts_with('#')) {
        let mut parts = line.splitn(2, ' ');
        if let (Some(sha), Some(name)) = (parts.next(), parts.next())
            && name.trim() == ref_name
        {
            return Some(sha.trim().to_owned());
        }
    }
    None
}

fn file_mtime_ms(path: &str) -> Option<u64> {
    fs::metadata(path)
        .ok()?
        .modified()
        .ok()?
        .duration_since(UNIX_EPOCH)
        .ok()
        .map(|d| d.as_millis() as u64)
}

fn format_timestamp(ms: u64) -> String {
    if ms == 0 {
        return "unknown".into();
    }
    let secs = ms / 1000;
    let days_since_epoch = (secs / 86400) as u32;
    let time_of_day = secs % 86400;

    let (year, month, day) = days_to_ymd(days_since_epoch);
    format!(
        "{:02}.{:02}.{} {:02}:{:02}:{:02} UTC",
        day,
        month,
        year,
        time_of_day / 3600,
        (time_of_day % 3600) / 60,
        time_of_day % 60
    )
}

fn days_to_ymd(mut days: u32) -> (u32, u32, u32) {
    days += 719468;
    let era = days / 146097;
    let doe = days % 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    (y, m, d)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_timestamp_zero() {
        assert_eq!(format_timestamp(0), "unknown");
    }

    #[test]
    fn test_format_timestamp_epoch() {
        // January 1, 1970 00:00:00 UTC
        assert_eq!(format_timestamp(1000), "01.01.1970 00:00:01 UTC");
    }

    #[test]
    fn test_format_timestamp_known_date() {
        // Test a known timestamp: 2024-01-15 12:30:45 UTC = 1705323045000 ms
        let result = format_timestamp(1705323045000);
        assert!(result.contains("2024"));
        assert!(result.contains("12:30:45"));
    }

    #[test]
    fn test_days_to_ymd_epoch() {
        // Day 0 in Unix epoch
        let (y, m, d) = days_to_ymd(0);
        assert_eq!((y, m, d), (1970, 1, 1));
    }

    #[test]
    fn test_days_to_ymd_various_dates() {
        // Test leap year date
        let (y, m, d) = days_to_ymd(365); // 1971-01-01
        assert_eq!((y, m, d), (1971, 1, 1));

        // Test known date
        let (y, m, d) = days_to_ymd(19723); // 2024-01-01
        assert_eq!((y, m, d), (2024, 1, 1));
    }

    #[test]
    fn test_is_numeric() {
        assert!(is_numeric("123"));
        assert!(is_numeric("0"));
        assert!(!is_numeric(""));
        assert!(!is_numeric("abc"));
        assert!(!is_numeric("12a"));
        assert!(!is_numeric("1.2"));
    }

    #[test]
    fn test_is_main_branch() {
        assert!(is_main_branch("main"));
        assert!(is_main_branch("master"));
        assert!(!is_main_branch("develop"));
        assert!(!is_main_branch("feature/test"));
        assert!(!is_main_branch(""));
    }

    #[test]
    fn test_git_info_default() {
        let info = GitInfo::default();
        assert_eq!(info.branch, "");
        assert_eq!(info.commit, "");
        assert_eq!(info.commit_short, "");
        assert_eq!(info.commit_time_ms, 0);
        assert!(!info.dirty);
    }

    #[test]
    fn test_format_timestamp_formatting() {
        // Test that formatting is correct for various timestamps
        let ts = 1609459200000; // 2021-01-01 00:00:00 UTC
        let result = format_timestamp(ts);
        assert!(result.contains("UTC"));
        assert!(result.contains("2021"));
    }

    #[test]
    fn test_format_timestamp_boundary() {
        // Test very small timestamp
        let result = format_timestamp(1);
        assert!(result.contains("1970"));

        // Test large timestamp (year 2099)
        let result = format_timestamp(4102444800000);
        assert!(result.contains("UTC"));
    }
}
