use super::types::ByteRange;

pub fn extract_attr_u64(line: &str, key: &str) -> Option<u64> {
    extract_attr_str(line, key)?.parse().ok()
}

pub fn extract_attr_str(line: &str, key: &str) -> Option<String> {
    let key_eq = format!("{}=", key);
    // Attributes follow #TAG: or a comma
    let pos = line
        .find(&format!(":{}", key_eq))
        .map(|p| p + 1)
        .or_else(|| line.find(&format!(",{}", key_eq)).map(|p| p + 1))?;

    let rest = &line[pos + key_eq.len()..];

    if rest.starts_with('"') {
        let end = rest[1..].find('"')?;
        Some(rest[1..1 + end].to_string())
    } else {
        let end = rest.find(',').unwrap_or(rest.len());
        Some(rest[..end].trim().to_string())
    }
}

pub fn resolve_url(base: &str, maybe_relative: &str) -> String {
    if maybe_relative.starts_with("http://") || maybe_relative.starts_with("https://") {
        return maybe_relative.to_string();
    }

    // Strip query string and fragment from base before resolving.
    // This prevents auth tokens (e.g. ?hdnts=...) from being embedded in the path.
    let base_clean = base.split('?').next().unwrap_or(base);
    let base_clean = base_clean.split('#').next().unwrap_or(base_clean);

    // Absolute path → replace host + path.
    if maybe_relative.starts_with('/') {
        if let Some(scheme_end) = base_clean.find("://") {
            let host_start = scheme_end + 3;
            let host_end = base_clean[host_start..]
                .find('/')
                .map(|p| host_start + p)
                .unwrap_or(base_clean.len());
            return format!("{}{}", &base_clean[..host_end], maybe_relative);
        }
    }

    // Relative path → strip last path component from base and append.
    let base_dir = base_clean.rfind('/').map(|i| &base_clean[..=i]).unwrap_or(base_clean);
    format!("{}{}", base_dir, maybe_relative)
}

pub fn parse_byte_range(attr: &str, last_end_offset: u64) -> ByteRange {
    let attr = attr.trim().trim_matches('"');
    let parts: Vec<&str> = attr.split('@').collect();
    let length = parts[0].trim().parse::<u64>().unwrap_or(0);
    let offset = if parts.len() > 1 {
        parts[1].trim().parse::<u64>().unwrap_or(0)
    } else {
        last_end_offset
    };
    ByteRange { length, offset }
}
