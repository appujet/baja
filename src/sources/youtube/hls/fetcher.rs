use super::types::Resource;

pub fn fetch_segment_into(
    client: &reqwest::blocking::Client,
    resource: &Resource,
    out: &mut Vec<u8>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let mut req = client.get(&resource.url).header("Accept", "*/*");

    if let Some(range) = &resource.range {
        let end = range.offset + range.length - 1;
        req = req.header("Range", format!("bytes={}-{}", range.offset, end));
    }

    let mut res = req.send()?;

    if !res.status().is_success() {
        return Err(format!("HLS fetch failed {}: {}", res.status(), resource.url).into());
    }

    let _n = res.copy_to(out)?;

    Ok(())
}
