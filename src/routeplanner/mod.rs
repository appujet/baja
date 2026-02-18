use crate::api::routeplanner::{FailingAddress, IpBlock, RotatingIpDetails, RoutePlannerStatus};
use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::Mutex;

#[async_trait]
pub trait RoutePlanner: Send + Sync {
    fn get_status(&self) -> RoutePlannerStatus;
    fn free_address(&self, address: &str);
    fn free_all_addresses(&self);
    fn mark_failed(&self, address: &str);
    fn get_address(&self) -> Option<std::net::IpAddr>;
}

pub struct RotatingIpRoutePlanner {
    ip_block: IpBlock,
    failing_addresses: Mutex<HashMap<String, u64>>,
    rotate_index: Mutex<u64>,
    ip_index: Mutex<u64>,
}

impl RotatingIpRoutePlanner {
    pub fn new(block_type: String, size: String) -> Self {
        Self {
            ip_block: IpBlock { block_type, size },
            failing_addresses: Mutex::new(HashMap::new()),
            rotate_index: Mutex::new(0),
            ip_index: Mutex::new(0),
        }
    }
}

#[async_trait]
impl RoutePlanner for RotatingIpRoutePlanner {
    fn get_status(&self) -> RoutePlannerStatus {
        let failing = self.failing_addresses.lock().unwrap();
        let failing_vec: Vec<FailingAddress> = failing
            .iter()
            .map(|(addr, ts)| FailingAddress {
                failing_address: addr.clone(),
                failing_timestamp: *ts,
                failing_time: "".to_string(), // Lavalink format usually human readable
            })
            .collect();

        RoutePlannerStatus::RotatingIpRoutePlanner(RotatingIpDetails {
            ip_block: self.ip_block.clone(),
            failing_addresses: failing_vec,
            rotate_index: self.rotate_index.lock().unwrap().to_string(),
            ip_index: self.ip_index.lock().unwrap().to_string(),
            current_address: "127.0.0.1".to_string(), // Placeholder for now
        })
    }

    fn free_address(&self, address: &str) {
        self.failing_addresses.lock().unwrap().remove(address);
    }

    fn free_all_addresses(&self) {
        self.failing_addresses.lock().unwrap().clear();
    }

    fn mark_failed(&self, address: &str) {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;
        self.failing_addresses
            .lock()
            .unwrap()
            .insert(address.to_string(), now);
    }

    fn get_address(&self) -> Option<std::net::IpAddr> {
        // Very basic rotation: for now we just return None unless we actually implement CIDR math
        // In a real implementation, we would take self.ip_block, parse it,
        // and add self.ip_index to the base address.
        None
    }
}
