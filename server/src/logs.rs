// SPDX-FileCopyrightText: 2026 Axle Duggan (axlecoffee) <contact@axle.coffee>
// SPDX-License-Identifier: AGPL-3.0-only
use mongodb::{
    Client, Collection, IndexModel,
    bson::{Document, doc},
    options::ClientOptions,
};
use std::time::Duration;
use tokio::sync::mpsc;

pub fn start(uri: String) -> (mpsc::Sender<Document>, tokio::task::JoinHandle<()>) {
    let (tx, mut rx) = mpsc::channel::<Document>(4096);
    let task = tokio::spawn(async move {
        let mut batch = Vec::with_capacity(64);
        let mut collection = None;
        loop {
            if rx.recv_many(&mut batch, 64).await == 0 {
                break;
            }
            if collection.is_none() {
                match connect(&uri).await {
                    Ok(connected) => collection = Some(connected),
                    Err(e) => {
                        tracing::error!("log store unreachable, dropped {}: {e:#}", batch.len());
                        batch.clear();
                        continue;
                    }
                }
            }
            let logs = collection.as_ref().unwrap();
            let result =
                tokio::time::timeout(Duration::from_secs(10), logs.insert_many(batch.iter())).await;
            match result {
                Ok(Ok(_)) => {}
                Ok(Err(e)) => {
                    tracing::error!("log write failed, dropped {}: {e:#}", batch.len());
                    collection = None;
                }
                Err(_) => {
                    tracing::error!("log write timed out, dropped {}", batch.len());
                    collection = None;
                }
            }
            batch.clear();
        }
    });
    (tx, task)
}

async fn connect(uri: &str) -> mongodb::error::Result<Collection<Document>> {
    let mut options = ClientOptions::parse(uri).await?;
    // auth and query options stay as the URI gave them, only the target database is ours
    options.default_database = Some("hst-api".into());
    options.server_selection_timeout = Some(Duration::from_secs(5));
    options.connect_timeout = Some(Duration::from_secs(5));
    options.max_pool_size = Some(4);
    options.app_name = Some("hst-api".into());

    let client = Client::with_options(options)?;
    let logs = client.database("hst-api").collection("request_logs");
    logs.create_indexes([
        IndexModel::builder().keys(doc! { "timestamp": -1 }).build(),
        IndexModel::builder()
            .keys(doc! { "country": 1, "timestamp": -1 })
            .build(),
    ])
    .await?;
    Ok(logs)
}
