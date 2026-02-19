use crate::configs::Config;
use tracing_subscriber::EnvFilter;

pub fn init(config: &Config) {
    // Determine the base log level
    let log_level = config
        .logging
        .as_ref()
        .and_then(|l| l.level.as_deref())
        .unwrap_or("info");

    // Get any additional filters
    let filters = config
        .logging
        .as_ref()
        .and_then(|l| l.filters.as_deref())
        .unwrap_or("");

    // Construct the filter string
    let filter_str = if filters.is_empty() {
        log_level.to_string()
    } else {
        format!("{},{}", log_level, filters)
    };

    // Create the environment filter, allowing RUST_LOG to override
    let env_filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(filter_str));

    // Initialize the subscriber
    tracing_subscriber::fmt()
        .with_env_filter(env_filter)
        .with_target(true) // Show module path (e.g., 'rustalink::server')
        .with_thread_ids(true) // Show thread ID for async context
        .with_line_number(true) // Show source line number
        .with_file(false) // Hide full file path to reduce clutter
        .init();
}
