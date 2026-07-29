use std::time::Duration;

use livtet_plugin::{protocol::MainToHost, transport::Transport};
use tokio::{process::Command, time::timeout};

/// Spawn `cat` as an echo process so we can round-trip a MessagePack message.
#[tokio::test]
async fn test_transport_round_trip() {
    let mut child = Command::new("cat")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .expect("failed to spawn cat");

    let stdin = child.stdin.take().unwrap();
    let stdout = child.stdout.take().unwrap();

    let mut transport = Transport::new(stdin, stdout);

    let msg = MainToHost::Shutdown;

    // Send
    transport.send(&msg).await.expect("send failed");

    // Receive (echoed back by cat)
    let received: MainToHost = timeout(Duration::from_secs(2), transport.recv())
        .await
        .expect("timed out waiting for response")
        .expect("recv failed");

    assert_eq!(received, msg);

    // Clean up the child
    let _ = child.start_kill();
}

/// Verify that send + recv preserves a more complex message.
#[tokio::test]
async fn test_transport_call_message() {
    let mut child = Command::new("cat")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .expect("failed to spawn cat");

    let stdin = child.stdin.take().unwrap();
    let stdout = child.stdout.take().unwrap();

    let mut transport = Transport::new(stdin, stdout);

    let msg = MainToHost::Call {
        id: "req-1".into(),
        plugin_id: "plugin-a".into(),
        capability: "read_file".into(),
        args: vec![serde_json::Value::String("/tmp/foo".into())],
    };

    transport.send(&msg).await.expect("send failed");

    let received: MainToHost = timeout(Duration::from_secs(2), transport.recv())
        .await
        .expect("timed out waiting for response")
        .expect("recv failed");

    assert_eq!(received, msg);

    let _ = child.start_kill();
}
