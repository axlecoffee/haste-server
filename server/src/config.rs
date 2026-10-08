// SPDX-FileCopyrightText: 2026 Axle Duggan (axlecoffee) <contact@axle.coffee>
// SPDX-License-Identifier: AGPL-3.0-only
use anyhow::{Context, Result, bail};
use std::{env, net::IpAddr};

pub struct Config {
    pub endpoint: String,
    pub region: String,
    pub bucket: String,
    pub access: String,
    pub secret: String,
    pub mongo: String,
    pub proxies: Vec<IpAddr>,
    pub assets: String,
    pub port: u16,
}

fn setting(key: &str, default: &str) -> String {
    env::var(key).unwrap_or_else(|_| default.into())
}

impl Config {
    pub fn load() -> Result<Self> {
        // Dotenv parser errors can contain credentials; never forward their text.
        if let Err(e) = dotenvy::dotenv() {
            if !e.not_found() { bail!("Invalid .env file"); }
        }
        let mongo = env::var("MONGODB_URI").context("Missing MONGODB_URI")?;
        if !mongo.starts_with("mongodb+srv://") {
            bail!("Mongo must use an SRV URI");
        }
        Ok(Self {
            endpoint: setting("S3_ENDPOINT", "https://rustfs.axle.coffee"),
            region: setting("S3_REGION", "ca-1"),
            bucket: setting("S3_BUCKET", "haste-store"),
            access: env::var("RUSTFS_ACCESS_KEY").map_err(|_| anyhow::anyhow!("Missing RUSTFS_ACCESS_KEY"))?,
            secret: env::var("RUSTFS_SECRET_KEY").map_err(|_| anyhow::anyhow!("Missing RUSTFS_SECRET_KEY"))?,
            mongo,
            proxies: setting("TRUSTED_PROXY_IPS", "")
                .split(',').filter(|s| !s.trim().is_empty())
                .map(|s| s.trim().parse().context("Invalid trusted proxy IP"))
                .collect::<Result<_>>()?,
            assets: setting("ASSET_DIR", "web/dist"),
            port: setting("PORT", "8292").parse().context("Invalid port")?,
        })
    }
}