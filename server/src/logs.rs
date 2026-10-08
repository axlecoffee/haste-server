// SPDX-FileCopyrightText: 2026 Axle Duggan (axlecoffee) <contact@axle.coffee>
// SPDX-License-Identifier: AGPL-3.0-only
use mongodb::{Client, IndexModel, bson::{Document, doc}, options::ClientOptions};
use std::time::Duration;
use tokio::sync::mpsc;

pub fn start(uri: String) -> (mpsc::Sender<Document>, tokio::task::JoinHandle<()>) {
    let (tx, mut rx) = mpsc::channel::<Document>(4096);
    let task = tokio::spawn(async move {
        let mut batch = Vec::with_capacity(64);
        let mut collection = None;
        loop {
            if rx.recv_many(&mut batch, 64).await == 0 { break; }
            let result = tokio::time::timeout(Duration::from_secs(10), async {
                if collection.is_none() {
                    let mut options = ClientOptions::parse(&uri).await?;
                    // Keep authentication and query options; only change the application database.
                    options.default_database = Some("hst-api".into());
                    options.server_selection_timeout = Some(Duration::from_secs(5));
                    options.connect_timeout = Some(Duration::from_secs(5));
                    options.max_pool_size = Some(4);
                    options.app_name = Some("hst-api".into());
                    let client = Client::with_options(options)?;
                    let logs = client.database("hst-api").collection::<Document>("request_logs");
                    logs.create_indexes([
                        IndexModel::builder().keys(doc! {"timestamp": -1}).build(),
                        IndexModel::builder().keys(doc! {"country": 1, "timestamp": -1}).build(),
                    ]).await?;
                    collection = Some(logs);
                }
                collection.as_ref().unwrap().insert_many(batch.iter()).await?;
                Ok::<_, mongodb::error::Error>(())
            }).await;
            if !matches!(result, Ok(Ok(()))) {
                tracing::error!(lost = batch.len(), "Mongo log persistence failed; batch dropped, service remains available");
                collection = None;
            }
            batch.clear();
        }
    });
    (tx, task)
}