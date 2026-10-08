// SPDX-FileCopyrightText: 2026 Axle Duggan (axlecoffee) <contact@axle.coffee>
// SPDX-License-Identifier: AGPL-3.0-only
use crate::config::Config;
use anyhow::{Result, bail};
use aws_sdk_s3::{Client, config::{BehaviorVersion, Credentials, Region}, primitives::ByteStream};
use rand::Rng;
use std::time::Duration;

pub struct Store {
    pub client: Client,
    pub bucket: String,
}

impl Store {
    pub fn new(config: &Config) -> Self {
        let client = Client::from_conf(aws_sdk_s3::config::Builder::new()
            .behavior_version(BehaviorVersion::latest())
            .endpoint_url(&config.endpoint)
            .region(Region::new(config.region.clone()))
            .credentials_provider(Credentials::new(&config.access, &config.secret, None, None, "external-file"))
            .force_path_style(true)
            .timeout_config(aws_sdk_s3::config::timeout::TimeoutConfig::builder()
                .operation_timeout(Duration::from_secs(20)).build())
            .build());
        Self { client, bucket: config.bucket.clone() }
    }

    pub async fn ready(&self, create: bool) -> Result<()> {
        if let Err(e) = self.client.head_bucket().bucket(&self.bucket).send().await {
            if create && e.raw_response().is_some_and(|r| r.status().as_u16() == 404) {
                self.client.create_bucket().bucket(&self.bucket)
                    .create_bucket_configuration(aws_sdk_s3::types::CreateBucketConfiguration::builder()
                        .location_constraint(aws_sdk_s3::types::BucketLocationConstraint::from("ca-1")).build())
                    .send().await.map_err(|_| anyhow::anyhow!("S3 bucket creation failed"))?;
            } else {
                bail!("S3 bucket unavailable (check endpoint, access and provisioning)");
            }
        }
        Ok(())
    }

    pub async fn get(&self, key: &str) -> Result<Option<String>> {
        match self.client.get_object().bucket(&self.bucket).key(key).send().await {
            Ok(object) => {
                let bytes = object.body.collect().await?;
                Ok(Some(String::from_utf8(bytes.into_bytes().to_vec())?))
            }
            Err(e) if e.as_service_error().is_some_and(|e| e.is_no_such_key()) => Ok(None),
            Err(_) => bail!("S3 read failed"),
        }
    }

    pub async fn put(&self, key: &str, text: &str) -> Result<bool> {
        match self.client.put_object().bucket(&self.bucket).key(key)
            .if_none_match("*").content_type("text/plain; charset=utf-8")
            .body(ByteStream::from(text.as_bytes().to_vec())).send().await {
            Ok(_) => Ok(true),
            Err(e) if e.raw_response().is_some_and(|r| matches!(r.status().as_u16(), 409 | 412)) => Ok(false),
            Err(_) => bail!("S3 write failed"),
        }
    }

    pub async fn save(&self, text: &str) -> Result<String> {
        for _ in 0..8 {
            let key = key();
            if self.put(&key, text).await? {
                return Ok(key);
            }
        }
        bail!("S3 key collision retry exhausted")
    }

    pub async fn probe(&self) -> Result<()> {
        let key = format!("_verification/{}", key());
        let result = async {
            if !self.put(&key, "conditional write probe").await? {
                bail!("Probe key already exists");
            }
            if self.put(&key, "must not overwrite").await? {
                bail!("RustFS does not enforce conditional writes");
            }
            if self.get(&key).await?.as_deref() != Some("conditional write probe") {
                bail!("Conditional write probe content mismatch");
            }
            Ok(())
        }.await;
        self.client.delete_object().bucket(&self.bucket).key(key).send().await
            .map_err(|_| anyhow::anyhow!("Could not remove verification object"))?;
        result
    }
}

fn key() -> String {
    let mut rng = rand::rng();
    (0..18).map(|i| {
        let alphabet: &[u8] = if i % 2 == 0 { b"bcdfghjklmnpqrstvwxyz" } else { b"aeiou" };
        alphabet[rng.random_range(0..alphabet.len())] as char
    }).collect()
}