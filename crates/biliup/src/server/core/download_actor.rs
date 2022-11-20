use crate::client::StatelessClient;
use crate::downloader::extractor::{find_extractor, SiteDefinition};
use crate::server::core::live_streamers::LiveStreamerDto;
use crate::server::core::util::{AnyMap, Cycle};
use crate::server::core::StreamStatus;
use anyhow::anyhow;
use indexmap::indexmap;
use std::any::Any;
use std::borrow::Borrow;
use std::collections::HashMap;
use std::ops::DerefMut;
use std::sync::{Arc, RwLock};
use std::time::Duration;
use tokio::task::JoinHandle;
use tracing::debug;

struct DownloadActor;

impl DownloadActor {
    fn new() -> Self {
        Self
    }

    async fn start_monitor(
        task: Cycle<StreamStatus>,
        extractor: &(dyn SiteDefinition + Send + Sync),
        client: StatelessClient,
    ) {
        let n = &mut 0;
        loop {
            let (url, status) = task.get(n);
            match (extractor.get_site(&url, client.clone()).await, status) {
                (Ok(site), StreamStatus::Idle) => {
                    println!("Idle\n {url} \n{site}");
                    task.write()
                        .entry(url)
                        .and_modify(|status| *status = StreamStatus::Downloading);
                }
                (Ok(site), StreamStatus::Downloading) => {
                    println!("Downloading {url}");
                }
                (Ok(site), StreamStatus::Pending) => {
                    println!("Pending");
                }
                (Ok(site), StreamStatus::Uploading) => {
                    println!("Uploading");
                }
                (Err(e), _) => {
                    debug!(url, "{e}")
                }
            }
            tokio::time::sleep(Duration::from_secs(30)).await;
        }
    }

    fn run(
        &mut self,
        list: Vec<LiveStreamerDto>,
        mut extensions: Arc<RwLock<AnyMap<(Cycle<StreamStatus>, JoinHandle<()>)>>>,
        client: StatelessClient,
    ) {
        for streamer in list {
            // let Some(extractor) = find_extractor(&streamer.url) else { continue; };
            let mut guard = extensions.write().unwrap();
            add_streamer(guard.deref_mut(), streamer.url, client.clone())
        }
        println!("{:?}", extensions);
    }
}

fn add_streamer(map: &mut AnyMap<(Cycle<StreamStatus>, JoinHandle<()>)>, url: String, client: StatelessClient) {
    let Some(extractor) = find_extractor(&url) else { return; };
    let entry = map
        .entry(extractor.as_any().type_id())
        .and_modify(|(cy, _)| cy.insert(url.clone(), StreamStatus::Idle))
        .or_insert_with(|| {
            let cycle = Cycle::new(indexmap![url => StreamStatus::Idle]);
            let task = cycle.clone();
            let handle = tokio::spawn(
                async move { DownloadActor::start_monitor(task, extractor, client).await },
            );
            (cycle, handle)
        });
}

pub struct DownloadActorHandle {
    platform_map: Arc<RwLock<AnyMap<(Cycle<StreamStatus>, JoinHandle<()>)>>>,
    client: StatelessClient,
}

impl DownloadActorHandle {
    pub fn new(list: Vec<LiveStreamerDto>, client: StatelessClient) -> Self {
        let mut actor = DownloadActor::new();
        let platform_map = Arc::new(RwLock::new(HashMap::default()));
        let platform = Arc::clone(&platform_map);
        let client_c = client.clone();
        actor.run(list, platform, client_c);
        Self {
            platform_map,
            client,
        }
    }

    pub fn add_streamer(&self, url: &str) {
        add_streamer(
            self.platform_map.write().unwrap().deref_mut(),
            url.to_string(),
            self.client.clone(),
        );
    }

    pub fn remove_streamer(&self, url: &str) {
        find_extractor(&url).and_then(|extractor| {
            self.platform_map
                .read()
                .unwrap()
                .get(&extractor.as_any().type_id())
                .and_then(|(cy, join_handle)| {
                    let mut guard = cy.write();
                    if guard.len() <= 1 {
                        join_handle.abort()
                    }
                    guard.remove(url)
                })
        });
    }
}
