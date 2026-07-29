use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    process::{ChildStdin, ChildStdout},
};

use crate::error::{PluginError, PluginResult};

pub struct Transport {
    stdin: ChildStdin,
    stdout: ChildStdout,
}

impl Transport {
    pub fn new(stdin: ChildStdin, stdout: ChildStdout) -> Self {
        Self { stdin, stdout }
    }

    pub async fn send<T: serde::Serialize>(&mut self, msg: &T) -> PluginResult<()> {
        let payload = rmp_serde::to_vec_named(msg).map_err(|e| PluginError::Ipc(e.to_string()))?;
        let len = (payload.len() as u32).to_le_bytes();
        self.stdin.write_all(&len).await?;
        self.stdin.write_all(&payload).await?;
        self.stdin.flush().await?;
        Ok(())
    }

    pub async fn recv<T: serde::de::DeserializeOwned>(&mut self) -> PluginResult<T> {
        let mut len_buf = [0u8; 4];
        self.stdout.read_exact(&mut len_buf).await?;
        let len = u32::from_le_bytes(len_buf) as usize;
        let mut payload = vec![0u8; len];
        self.stdout.read_exact(&mut payload).await?;
        rmp_serde::from_slice(&payload).map_err(|e| PluginError::Ipc(e.to_string()))
    }
}
