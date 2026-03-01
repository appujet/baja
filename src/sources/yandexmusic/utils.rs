use md5::{Digest, Md5};

pub fn generate_download_sign(path: &str, s: &str) -> String {
    let sign = format!("XGRlBW9FXlekgbPrRHuSiA{}{}", path, s);
    let mut hasher = Md5::new();
    hasher.update(sign.as_bytes());
    let result = hasher.finalize();
    format!("{:x}", result)
}
