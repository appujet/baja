use super::types::Resource;
use crate::common::types::AnyResult;

pub async fn fetch_segment_into(
  client: &reqwest::Client,
  resource: &Resource,
  out: &mut Vec<u8>,
) -> AnyResult<()> {
  let mut req = client.get(&resource.url).header("Accept", "*/*");

  if let Some(range) = &resource.range {
    let end = range.offset + range.length - 1;
    req = req.header("Range", format!("bytes={}-{}", range.offset, end));
  }

  let res = req.send().await?;

  if !res.status().is_success() {
    return Err(format!("HLS fetch failed {}: {}", res.status(), resource.url).into());
  }

  let bytes = res.bytes().await?;
  out.extend_from_slice(&bytes);

  Ok(())
}
