use std::{
    collections::HashMap,
    net::{IpAddr, Ipv4Addr, Ipv6Addr},
    str::FromStr,
    sync::Mutex,
};

use async_trait::async_trait;
use ipnet::IpNet;

use crate::api::{
    BalancingIpDetails,
    routeplanner::{FailingAddress, IpBlock, RotatingIpDetails, RoutePlannerStatus},
};

#[async_trait]
pub trait RoutePlanner: Send + Sync {
    fn get_status(&self) -> RoutePlannerStatus;
    fn free_address(&self, address: &str);
    fn free_all_addresses(&self);
    fn mark_failed(&self, address: &str);
    fn get_address(&self) -> Option<std::net::IpAddr>;
}

pub struct BalancingIpRoutePlanner {
    ip_blocks: Vec<IpBlock>,
    parsed_blocks: Vec<IpNet>,
    failing_addresses: Mutex<HashMap<String, u64>>,
    block_index: Mutex<usize>,
    ip_indices: Mutex<Vec<u128>>,
}

impl BalancingIpRoutePlanner {
    pub fn new(cidrs: Vec<String>) -> Self {
        let mut ip_blocks = Vec::new();
        let mut parsed_blocks = Vec::new();
        let mut ip_indices = Vec::new();

        for cidr in cidrs {
            let parsed = IpNet::from_str(&cidr)
                .or_else(|_| {
                    // If parsing fails, try adding /32 or /128
                    if cidr.contains(':') {
                        IpNet::from_str(&format!("{}/128", cidr))
                    } else {
                        IpNet::from_str(&format!("{}/32", cidr))
                    }
                })
                .unwrap_or_else(|e| {
                    panic!("Invalid CIDR or IP '{}' for route planner: {}", cidr, e);
                });

            let block_type = match parsed {
                IpNet::V4(_) => "Inet4Address".to_string(),
                IpNet::V6(_) => "Inet6Address".to_string(),
            };

            ip_blocks.push(IpBlock {
                block_type,
                size: cidr,
            });
            parsed_blocks.push(parsed);
            ip_indices.push(0);
        }

        Self {
            ip_blocks,
            parsed_blocks,
            failing_addresses: Mutex::new(HashMap::new()),
            block_index: Mutex::new(0),
            ip_indices: Mutex::new(ip_indices),
        }
    }

    fn next_ip(&self) -> IpAddr {
        let mut b_idx = self.block_index.lock().unwrap_or_else(|e| e.into_inner());
        let mut indices = self.ip_indices.lock().unwrap_or_else(|e| e.into_inner());

        // Pick current block
        let block_idx = *b_idx % self.parsed_blocks.len();
        let block = &self.parsed_blocks[block_idx];

        // Calculate increment for this block
        let prefix_len = block.prefix_len();
        let max_bits = if let IpNet::V4(_) = block { 32 } else { 128 };
        let size_bits = max_bits - prefix_len;

        let increment = if size_bits > 7 {
            use rand::Rng;
            let mut rng = rand::thread_rng();
            (rng.gen_range(0..10) + 10) as u128
        } else {
            1
        };

        let current_index = indices[block_idx];
        indices[block_idx] = current_index.wrapping_add(increment);
        let final_index = indices[block_idx];

        // Move to next block for next call
        *b_idx = (*b_idx + 1) % self.parsed_blocks.len();

        match block {
            IpNet::V4(net) => {
                let addr_u32 = u32::from(net.addr());
                let offset = if prefix_len >= 32 {
                    0
                } else if prefix_len == 0 {
                    final_index as u32
                } else {
                    (final_index as u32) & (!0u32 >> prefix_len)
                };
                IpAddr::V4(Ipv4Addr::from(addr_u32 + offset))
            }
            IpNet::V6(net) => {
                let addr_u128 = u128::from(net.addr());
                let offset = if prefix_len >= 128 {
                    0
                } else if prefix_len == 0 {
                    final_index
                } else {
                    final_index & (!0u128 >> prefix_len)
                };
                IpAddr::V6(Ipv6Addr::from(addr_u128 + offset))
            }
        }
    }

    fn get_address_internal(&self) -> Option<IpAddr> {
        let mut tries = 0;
        loop {
            if tries > 100 {
                return None;
            }
            tries += 1;

            let ip = self.next_ip();
            let ip_str = ip.to_string();
            let is_failing = {
                let mut failing = self
                    .failing_addresses
                    .lock()
                    .unwrap_or_else(|e| e.into_inner());
                if let Some(&timestamp) = failing.get(&ip_str) {
                    let now = crate::server::app_state::now_ms();
                    if now > timestamp + 604800000 {
                        failing.remove(&ip_str);
                        false
                    } else {
                        true
                    }
                } else {
                    false
                }
            };

            if !is_failing {
                return Some(ip);
            }
        }
    }
}

#[async_trait]
impl RoutePlanner for BalancingIpRoutePlanner {
    fn get_status(&self) -> RoutePlannerStatus {
        let failing = self
            .failing_addresses
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let failing_vec: Vec<FailingAddress> = failing
            .iter()
            .map(|(addr, ts)| FailingAddress {
                failing_address: addr.clone(),
                failing_timestamp: *ts,
                failing_time: "".to_string(),
            })
            .collect();

        // If only 1 block, return Rotating details for compatibility
        if self.ip_blocks.len() == 1 {
            let index = self.ip_indices.lock().unwrap_or_else(|e| e.into_inner())[0];
            let block = &self.parsed_blocks[0];
            let current_ip = match block {
                IpNet::V4(net) => {
                    let addr_u32 = u32::from(net.addr());
                    let offset = (index as u32) & (!0 >> net.prefix_len());
                    IpAddr::V4(Ipv4Addr::from(addr_u32 + offset)).to_string()
                }
                IpNet::V6(net) => {
                    let addr_u128 = u128::from(net.addr());
                    let mask = !0u128 >> net.prefix_len();
                    let offset = index & mask;
                    IpAddr::V6(Ipv6Addr::from(addr_u128 + offset)).to_string()
                }
            };

            RoutePlannerStatus::RotatingIpRoutePlanner(RotatingIpDetails {
                ip_block: self.ip_blocks[0].clone(),
                failing_addresses: failing_vec,
                rotate_index: "0".to_string(),
                ip_index: index.to_string(),
                current_address: current_ip,
            })
        } else {
            // Pick a reasonable summary block or combine them
            RoutePlannerStatus::BalancingIpRoutePlanner(BalancingIpDetails {
                ip_block: self.ip_blocks[0].clone(), // Simplified
                failing_addresses: failing_vec,
            })
        }
    }

    fn free_address(&self, address: &str) {
        self.failing_addresses
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(address);
    }

    fn free_all_addresses(&self) {
        self.failing_addresses
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clear();
    }

    fn mark_failed(&self, address: &str) {
        let now = crate::server::app_state::now_ms();
        self.failing_addresses
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(address.to_string(), now);
    }

    fn get_address(&self) -> Option<std::net::IpAddr> {
        self.get_address_internal()
    }
}
