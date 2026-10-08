// SPDX-FileCopyrightText: 2026 Axle Duggan (axlecoffee) <contact@axle.coffee>
// SPDX-License-Identifier: AGPL-3.0-only
use anyhow::{Context, Result, anyhow, bail};
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

// VarError can quote the value itself, so only the key name reaches the user
fn require(key: &str) -> Result<String> {
    env::var(key).map_err(|_| anyhow!("Missing {key}"))
}

impl Config {
    pub fn load() -> Result<Self> {
        // dotenvy parser errors can quote credential lines, never forward their text
        if let Err(e) = dotenvy::dotenv()
            && !e.not_found()
        {
            bail!("Invalid .env file");
        }

        let mongo = require("MONGODB_URI")?;
        if !mongo.starts_with("mongodb+srv://") {
            bail!("Mongo must use an SRV URI");
        }

        let mut proxies = Vec::new();
        for entry in setting("TRUSTED_PROXY_IPS", "").split(',') {
            let entry = entry.trim();
            if !entry.is_empty() {
                proxies.push(entry.parse().context("Invalid trusted proxy IP")?);
            }
        }

        Ok(Self {
            endpoint: setting("S3_ENDPOINT", "https://rustfs.axle.coffee"),
            region: setting("S3_REGION", "ca-1"),
            bucket: setting("S3_BUCKET", "haste-store"),
            access: require("RUSTFS_ACCESS_KEY")?,
            secret: require("RUSTFS_SECRET_KEY")?,
            mongo,
            proxies,
            assets: setting("ASSET_DIR", "web/dist"),
            port: setting("PORT", "8292").parse().context("Invalid port")?,
        })
    }
}
