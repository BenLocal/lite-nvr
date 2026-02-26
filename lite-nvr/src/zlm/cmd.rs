use std::sync::OnceLock;

static ZLM_CMD_SENDER: OnceLock<tokio::sync::mpsc::Sender<ZlmCmd>> = OnceLock::new();

pub enum ZlmCmd {}

pub(crate) fn init_zlm_cmd_sender() -> anyhow::Result<tokio::sync::mpsc::Receiver<ZlmCmd>> {
    let (tx, rx) = tokio::sync::mpsc::channel(1024);
    ZLM_CMD_SENDER
        .set(tx)
        .map_err(|e| anyhow::anyhow!("Failed to set ZLM_CMD_SENDER: {:?}", e))?;
    Ok(rx)
}

pub(crate) fn blocking_send_cmd(cmd: ZlmCmd) -> anyhow::Result<()> {
    ZLM_CMD_SENDER
        .get()
        .ok_or(anyhow::anyhow!("ZLM_CMD_SENDER not initialized"))
        .and_then(|sender| {
            sender
                .blocking_send(cmd)
                .map_err(|_| anyhow::anyhow!("Failed to send ZLM_CMD"))
        })
        .map(|_| ())
}

pub(crate) fn handler_zlm_cmd(cmd: ZlmCmd) -> anyhow::Result<()> {
    Ok(())
}
