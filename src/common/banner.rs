const GREEN: &str = "\x1b[32m";
const CYAN: &str = "\x1b[36m";
const YELLOW: &str = "\x1b[33m";
const BOLD: &str = "\x1b[1m";
const RESET: &str = "\x1b[0m";
const DIM: &str = "\x1b[2m";

macro_rules! env_or {
    ($key:literal, $default:literal) => {
        option_env!($key).unwrap_or($default)
    };
}

pub struct BannerInfo {
    pub version: &'static str,
    pub build_time: &'static str,
    pub branch: &'static str,
    pub commit: &'static str,
    pub commit_short: &'static str,
    pub commit_time: &'static str,
    pub rust_version: &'static str,
    pub dirty: bool,
    pub profile: &'static str,
}

impl Default for BannerInfo {
    fn default() -> Self {
        Self {
            version: env!("CARGO_PKG_VERSION"),
            build_time: env_or!("BUILD_TIME_HUMAN", "unknown"),
            branch: env_or!("GIT_BRANCH", "unknown"),
            commit: env_or!("GIT_COMMIT", "unknown"),
            commit_short: env_or!("GIT_COMMIT_SHORT", "unknown"),
            commit_time: env_or!("GIT_COMMIT_TIME_HUMAN", "unknown"),
            rust_version: env_or!("RUST_VERSION", "unknown"),
            dirty: matches!(option_env!("GIT_DIRTY"), Some("true")),
            profile: if cfg!(debug_assertions) {
                "debug"
            } else {
                "release"
            },
        }
    }
}

pub fn print_banner(info: &BannerInfo) {
    println!();
    println!("{GREEN}    ____            __        ___       __  {RESET}");
    println!("{GREEN}   / __ \\__  _______/ /_____ _/ (_)___  / /__  {RESET}");
    println!("{GREEN}  / /_/ / / / / ___/ __/ __ `/ / / __ \\/ //_/  {RESET}");
    println!("{GREEN} / _, _/ /_/ (__  ) /_/ /_/ / / / / / / ,<     {RESET}");
    println!("{GREEN}/_/ |_|\\__,_/____/\\__/\\__,_/_/_/_/ /_/_/|_|  {RESET}");
    println!("{DIM}========================================{RESET}");
    println!();

    print_row("Version", info.version, CYAN);
    print_row("Build time", info.build_time, RESET);
    print_row("Branch", info.branch, RESET);

    let commit_display = if info.dirty {
        format!("{}{YELLOW} (dirty){RESET}", info.commit_short)
    } else {
        info.commit_short.to_owned()
    };
    print_row_owned("Commit", &commit_display);
    print_row("Commit time", info.commit_time, RESET);
    print_row("Rust", info.rust_version, RESET);
    print_row("Profile", info.profile, YELLOW);

    println!();
    println!(
        "{DIM}  No active profile set, falling back to 1 default profile: \
         \"{BOLD}default{RESET}{DIM}\"{RESET}"
    );
    println!();
}

fn print_row(label: &str, value: &'static str, color: &str) {
    println!("  {BOLD}{label:<14}{RESET}{color}{value}{RESET}");
}

fn print_row_owned(label: &str, value: &str) {
    println!("  {BOLD}{label:<14}{RESET}{value}");
}
