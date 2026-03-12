use std::{
    env, fs,
    path::Path,
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};

/// Collects build-time and Git metadata and emits Cargo environment directives for the build.
///
/// This function configures Cargo rerun triggers, captures the current timestamp and Rust compiler
/// version, gathers Git metadata (branch, commit, short commit, commit time, and dirty state),
/// and emits those values as `cargo:rustc-env` variables. If a pre-release identifier is detected,
/// it is emitted as `RUSTALINK_PRE_RELEASE`.
///
/// Emitted environment variables include:
/// - `BUILD_TIME` (milliseconds since UNIX epoch)
/// - `BUILD_TIME_HUMAN` (formatted UTC timestamp)
/// - `RUST_VERSION`
/// - `GIT_BRANCH`, `GIT_COMMIT`, `GIT_COMMIT_SHORT`, `GIT_COMMIT_TIME`, `GIT_COMMIT_TIME_HUMAN`, `GIT_DIRTY`, `GIT_VERSION_STRING`
/// - `RUSTALINK_PRE_RELEASE` (when detected)
///
/// # Examples
///
/// ```
/// // Typical invocation from a build script entrypoint.
/// // Running this in a doctest will simply call the function; side effects set environment
/// // variables via Cargo's build script protocol and are not visible here.
/// let _ = main();
/// ```
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

/// Register Cargo rerun directives so the build script is re-run when Git HEAD, branch refs (if present),
/// or common CI environment variables indicating the commit/branch change.
///
/// # Examples
///
/// ```
/// // Call near the start of build.rs to ensure Cargo rebuilds when relevant Git state or CI env vars change.
/// setup_rerun_triggers();
/// ```
fn setup_rerun_triggers() {
    println!("cargo:rerun-if-changed=.git/HEAD");
    if Path::new(".git/refs/heads").exists() {
        println!("cargo:rerun-if-changed=.git/refs/heads");
    }
    println!("cargo:rerun-if-env-changed=GITHUB_SHA");
    println!("cargo:rerun-if-env-changed=GITHUB_REF_NAME");
    println!("cargo:rerun-if-env-changed=GITHUB_REF");
}

/// Emit a Cargo directive that sets an environment variable for the compiler.
///
/// Prints a line of the form `cargo:rustc-env=NAME=VALUE` to stdout so Cargo will
/// make `NAME` available as an environment variable to the compiled crate.
///
/// # Examples
///
/// ```
/// emit_env("BUILD_TIME", "2026-03-12T00:00:00Z");
/// ```
fn emit_env<V: std::fmt::Display>(name: &str, value: V) {
    println!("cargo:rustc-env={}={}", name, value);
}

/// Get the current `rustc` version string.

///

/// Queries the `rustc --version` command and returns its trimmed output. If invoking `rustc` fails,

/// falls back to the `RUSTC` environment variable, and if that is unset returns `"unknown"`.

///

/// # Examples

///

/// ```

/// let ver = get_rustc_version();

/// assert!(!ver.is_empty());

/// ```

///

/// # Returns

///

/// A `String` containing the version reported by `rustc`, the `RUSTC` env value, or `"unknown"`.
fn get_rustc_version() -> String {
    Command::new("rustc")
        .arg("--version")
        .output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_owned())
        .unwrap_or_else(|| env::var("RUSTC").unwrap_or_else(|_| "unknown".into()))
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
    /// Collects Git metadata and returns a populated `GitInfo`.
    ///
    /// Attempts to populate fields from CI/CD environment variables (`GITHUB_REF_NAME`, `GITHUB_SHA`),
    /// falls back to running `git` commands to determine branch, commit (and short commit), commit
    /// timestamp, and repository dirtiness, and finally uses `.git` file parsing and ref modification
    /// times as additional fallbacks when git commands are unavailable.
    ///
    /// The resulting `GitInfo` contains `branch`, `commit`, `commit_short`, `commit_time_ms`, and
    /// `dirty` set according to the best available source.
    ///
    /// # Examples
    ///
    /// ```
    /// let info = GitInfo::gather();
    /// // Fields may be "unknown" if no git data is available in the environment or repository.
    /// let _branch = info.branch;
    /// let _commit = info.commit;
    /// ```
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

    /// Emits Git-related metadata as `cargo:rustc-env` environment variables for the build.
    ///
    /// The following variables are emitted: `GIT_BRANCH`, `GIT_COMMIT`, `GIT_COMMIT_SHORT`,
    /// `GIT_COMMIT_TIME`, `GIT_COMMIT_TIME_HUMAN`, `GIT_DIRTY`, and a derived `GIT_VERSION_STRING`
    /// combining branch, short commit, and a `-dirty` suffix when applicable.
    ///
    /// # Examples
    ///
    /// ```
    /// let info = GitInfo {
    ///     branch: "main".into(),
    ///     commit: "0123456789abcdef".into(),
    ///     commit_short: "0123456".into(),
    ///     commit_time_ms: 1_600_000_000_000,
    ///     dirty: false,
    /// };
    /// info.emit(); // sets the corresponding cargo:rustc-env variables for the build
    /// ```
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

/// Determines a pre-release identifier derived from CI environment variables or local Git metadata.
///
/// The function checks sources in the following priority order and returns the first usable identifier:
/// 1. `GITHUB_REF_NAME` — substring after the first `-` or the whole non-main, non-`v`-prefixed branch name.
/// 2. `GITHUB_REF` — substring after the last `-` (typical GitHub full ref format).
/// 3. `git describe --tags --always --dirty` — extracts a non-numeric pre-release segment when present.
/// 4. Local branch name from `git rev-parse --abbrev-ref HEAD` if it is not `main`, `master`, or `HEAD`.
///
/// # Returns
///
/// `Some(String)` with the detected pre-release identifier, `None` if no identifier could be determined.
///
/// # Examples
///
/// ```
/// let _ = detect_pre_release();
/// ```
fn detect_pre_release() -> Option<String> {
    // Priority 1: GITHUB_REF_NAME (tag or branch)
    if let Ok(v) = env::var("GITHUB_REF_NAME") {
        if let Some(idx) = v.find('-') {
            return Some(v[idx + 1..].to_string());
        }
        // Use non-main branches as pre-release identifiers
        if !is_main_branch(&v) && !v.starts_with('v') {
            return Some(v);
        }
    }

    // Priority 2: GITHUB_REF (standard tag format)
    if let Ok(v) = env::var("GITHUB_REF")
        && let Some(idx) = v.rfind('-')
    {
        return Some(v[idx + 1..].to_string());
    }

    // Priority 3: Git describe
    if let Some(desc) = git_output(&["describe", "--tags", "--always", "--dirty"])
        && let Some(idx) = desc.find('-')
    {
        let part = &desc[idx + 1..];
        // Handle cases like v1.0.8-beta.1-2-gabc123
        if let Some(next_dash) = part.find('-') {
            let pre = &part[..next_dash];
            if !is_numeric(pre) {
                return Some(pre.to_string());
            }
        } else if !is_numeric(part) {
            return Some(part.to_string());
        }
    }

    // Priority 4: Local branch name
    if let Some(branch) = git_output(&["rev-parse", "--abbrev-ref", "HEAD"])
        && !is_main_branch(&branch)
        && branch != "HEAD"
        && !branch.is_empty()
    {
        return Some(branch);
    }

    None
}

/// Determines whether a branch name refers to the canonical primary branch.
///
/// # Returns
/// `true` if the name is "main" or "master", `false` otherwise.
///
/// # Examples
///
/// ```
/// assert!(is_main_branch("main"));
/// assert!(is_main_branch("master"));
/// assert!(!is_main_branch("feature/x"));
/// ```
fn is_main_branch(name: &str) -> bool {
    matches!(name, "main" | "master")
}

/// Checks whether a string consists solely of numeric characters.
///
/// # Returns
///
/// `true` if the string contains at least one character and every character is numeric, `false` otherwise.
///
/// # Examples
///
/// ```
/// assert!(is_numeric("0"));
/// assert!(is_numeric("123456"));
/// assert!(!is_numeric(""));
/// assert!(!is_numeric("12a34"));
/// ```
fn is_numeric(s: &str) -> bool {
    !s.is_empty() && s.chars().all(char::is_numeric)
}

/// Run `git` with the given arguments and return its trimmed standard output on success.
///
/// `args` are the arguments passed to the `git` executable (e.g., `["rev-parse", "HEAD"]`).
///
/// # Returns
///
/// `Some(String)` containing the trimmed stdout if the `git` command exits successfully, `None` otherwise.
///
/// # Examples
///
/// ```
/// if let Some(head) = git_output(&["rev-parse", "HEAD"]) {
///     assert!(!head.is_empty());
/// }
/// ```
fn git_output(args: &[&str]) -> Option<String> {
    let out = Command::new("git").args(args).output().ok()?;
    if out.status.success() {
        Some(String::from_utf8_lossy(&out.stdout).trim().to_owned())
    } else {
        None
    }
}

/// Reads `.git/HEAD` and resolves the current branch name and commit SHA when available.
///
/// If `.git/HEAD` contains a ref (`ref: ...`), returns `Some((branch, commit))` where `branch` is
/// the last path segment of the ref (e.g., `main`) and `commit` is the SHA read from the ref file
/// or from packed-refs; `commit` will be an empty string if no SHA can be found. If `.git/HEAD` is
/// not a ref, returns `Some(("HEAD", <contents>))` where `<contents>` is the HEAD value. Returns
/// `None` if `.git/HEAD` cannot be read.
///
/// # Examples
///
/// ```
/// if let Some((branch, commit)) = parse_dot_git_head() {
///     // handle branch/commit (commit may be empty if not found)
///     let _ = (branch, commit);
/// }
/// ```
fn parse_dot_git_head() -> Option<(String, String)> {
    let head = fs::read_to_string(".git/HEAD").ok()?.trim().to_owned();

    if let Some(ref_path) = head.strip_prefix("ref: ") {
        let branch = ref_path
            .split('/')
            .next_back()
            .unwrap_or("unknown")
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

/// Look up a reference name in `.git/packed-refs` and return its object SHA if present.
///
/// The `ref_name` should be the full ref path (for example `"refs/heads/main"` or
/// `"refs/tags/v1.2.3"`). The function reads `.git/packed-refs`, ignores comment lines,
/// and returns the corresponding SHA as a trimmed hexadecimal string when a matching ref is found.
///
/// # Returns
/// `Some(String)` containing the SHA for the given ref name if found, `None` otherwise.
///
/// # Examples
///
/// ```no_run
/// let sha = packed_ref_lookup("refs/heads/main");
/// if let Some(sha) = sha {
///     println!("Main branch SHA: {}", sha);
/// }
/// ```
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

/// Returns the file's modification time as milliseconds since the UNIX epoch.
///
/// Attempts to read filesystem metadata for `path` and convert the file's last
/// modification time to milliseconds. Returns `None` if the metadata or time
/// cannot be obtained.
///
/// # Examples
///
/// ```
/// let ms = file_mtime_ms("Cargo.toml");
/// if let Some(t) = ms {
///     assert!(t > 0);
/// }
/// ```
fn file_mtime_ms(path: &str) -> Option<u64> {
    fs::metadata(path)
        .ok()?
        .modified()
        .ok()?
        .duration_since(UNIX_EPOCH)
        .ok()
        .map(|d| d.as_millis() as u64)
}

/// Format a millisecond Unix timestamp into a human-readable UTC string.
///
/// Returns `"unknown"` if `ms` is zero. Otherwise returns a string in the
/// format `DD.MM.YYYY HH:MM:SS UTC`.
///
/// # Examples
///
/// ```
/// assert_eq!(format_timestamp(0), "unknown");
/// // 1970-01-02 03:04:05 UTC -> 1 day + 03:04:05 = 86_400 + 11_045 = 97_445 seconds
/// let ms = 97_445_000u64;
/// assert_eq!(format_timestamp(ms), "02.01.1970 03:04:05 UTC");
/// ```
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

/// Converts a day count (relative to an internal proleptic Gregorian epoch) to a calendar date.
///
/// The function maps an input number of days to a (year, month, day) tuple using a
/// pure arithmetic algorithm that produces the corresponding proleptic Gregorian date.
///
/// # Examples
///
/// ```
/// let (y, m, d) = days_to_ymd(0);
/// // Year, month and day are valid calendar components.
/// assert!(m >= 1 && m <= 12);
/// assert!(d >= 1 && d <= 31);
/// ```
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
