// SPDX-FileCopyrightText: 2026 Axle Duggan (axlecoffee) <contact@axle.coffee>
// SPDX-License-Identifier: AGPL-3.0-only
use std::{collections::HashMap, net::IpAddr};

#[derive(Default)]
pub struct Limiter {
    clients: HashMap<IpAddr, (u64, u32)>,
    prune_at: u64,
}

impl Limiter {
    pub fn check(&mut self, ip: IpAddr, now: u64) -> Option<(bool, u32, u64)> {
        if now >= self.prune_at {
            self.clients.retain(|_, (reset, _)| *reset > now);
            self.prune_at = now + 60;
        }
        if self.clients.len() >= 100_000 && !self.clients.contains_key(&ip) {
            return None;
        }
        let (reset, count) = self.clients.entry(ip).or_insert((now + 60, 0));
        if now >= *reset { *reset = now + 60; *count = 0; }
        let allowed = *count < 120;
        if allowed { *count += 1; }
        Some((allowed, 120 - *count, *reset))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rate_limit_is_per_ip_and_resets() {
        let mut limiter = Limiter::default();
        let ip = "127.0.0.1".parse().unwrap();
        for i in 1..=120 { assert_eq!(limiter.check(ip, 100), Some((true, 120 - i, 160))); }
        assert_eq!(limiter.check(ip, 159), Some((false, 0, 160)));
        assert_eq!(limiter.check("127.0.0.2".parse().unwrap(), 159), Some((true, 119, 219)));
        assert_eq!(limiter.check(ip, 160), Some((true, 119, 220)));
    }
}