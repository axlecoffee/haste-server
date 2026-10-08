// SPDX-FileCopyrightText: 2026 Axle Duggan (axlecoffee) <contact@axle.coffee>
// SPDX-License-Identifier: AGPL-3.0-only
use std::{collections::HashMap, net::IpAddr};

pub struct Quota {
    pub allowed: bool,
    pub remaining: u32,
    pub reset: u64,
}

#[derive(Default)]
pub struct Limiter {
    clients: HashMap<IpAddr, (u64, u32)>,
    prune_at: u64,
}

impl Limiter {
    // fixed 60s window per client ip, 120 creates a minute
    pub fn check(&mut self, ip: IpAddr, now: u64) -> Option<Quota> {
        if now >= self.prune_at {
            self.clients.retain(|_, (reset, _)| *reset > now);
            self.prune_at = now + 60;
        }
        if self.clients.len() >= 100_000 && !self.clients.contains_key(&ip) {
            return None;
        }
        let (reset, count) = self.clients.entry(ip).or_insert((now + 60, 0));
        if now >= *reset {
            *reset = now + 60;
            *count = 0;
        }
        let allowed = *count < 120;
        if allowed {
            *count += 1;
        }
        Some(Quota {
            allowed,
            remaining: 120 - *count,
            reset: *reset,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn limits_are_per_ip_and_reset_each_minute() {
        let mut limiter = Limiter::default();
        let ip = "127.0.0.1".parse().unwrap();
        let other = "127.0.0.2".parse().unwrap();

        for i in 1..=120 {
            let quota = limiter.check(ip, 100).unwrap();
            assert!(quota.allowed);
            assert_eq!(quota.remaining, 120 - i);
        }
        assert!(!limiter.check(ip, 159).unwrap().allowed);

        // another ip has its own budget
        let quota = limiter.check(other, 159).unwrap();
        assert!(quota.allowed);
        assert_eq!(quota.remaining, 119);

        // the window rolls over
        let quota = limiter.check(ip, 160).unwrap();
        assert!(quota.allowed);
        assert_eq!(quota.remaining, 119);
        assert_eq!(quota.reset, 220);
    }
}
