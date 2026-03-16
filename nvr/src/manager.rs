use std::{
    collections::HashMap,
    sync::{Arc, LazyLock},
};

use tokio::sync::RwLock;

use crate::media::{pipe::Pipe, types::PipeConfig};

static PIPE_MANAGER: LazyLock<RwLock<HashMap<String, Arc<Pipe>>>> =
    LazyLock::new(|| RwLock::new(HashMap::new()));

pub(crate) fn get_pipe_manager() -> &'static RwLock<HashMap<String, Arc<Pipe>>> {
    &PIPE_MANAGER
}

pub(crate) async fn add_pipe(
    id: &str,
    config: PipeConfig,
    update_if_exists: bool,
) -> anyhow::Result<()> {
    let mut pipes = PIPE_MANAGER.write().await;
    if pipes.contains_key(id) {
        if !update_if_exists {
            return Err(anyhow::anyhow!("Pipe already exists"));
        } else {
            if let Some(pipe) = pipes.remove(id) {
                pipe.cancel();
            }
        }
    }
    let pipe = Arc::new(Pipe::new(config));
    pipes.insert(id.to_string(), Arc::clone(&pipe));

    tokio::spawn(async move {
        pipe.start().await;
    });
    Ok(())
}

pub(crate) async fn remove_pipe(id: &str) -> anyhow::Result<()> {
    let mut pipes = PIPE_MANAGER.write().await;
    if let Some(pipe) = pipes.remove(id) {
        pipe.cancel();
    }
    Ok(())
}

pub(crate) async fn get_pipe(id: &str) -> Option<Arc<Pipe>> {
    PIPE_MANAGER.read().await.get(id).cloned()
}
