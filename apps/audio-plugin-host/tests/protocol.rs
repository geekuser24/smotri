//! No third-party binaries needed: exercise the real disposable helper on every platform.
use filmcraft_audio_plugin_client::{PROTOCOL, receive, send};
use filmcraft_project::audio_plugin::{PluginRequest, PluginResponse};
use std::{
    io::Write,
    net::TcpListener,
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};
struct Guard(Child);
impl Drop for Guard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
#[test]
fn helper_handshake_errors_and_oversized_packet_are_contained() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let nonce = "ab".repeat(32);
    let mut child = Guard(
        Command::new(env!("CARGO_BIN_EXE_filmcraft-audio-plugin-host"))
            .args(["--worker", &listener.local_addr().unwrap().port().to_string()])
            .env("SMOTRI_PLUGIN_NONCE", &nonce)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap(),
    );
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut stream = loop {
        if let Ok((s, _)) = listener.accept() {
            break s;
        }
        assert!(Instant::now() < deadline && child.0.try_wait().unwrap().is_none(), "helper failed to connect");
        std::thread::sleep(Duration::from_millis(10));
    };
    // macOS accepts inherit O_NONBLOCK from the listener, just as in the client.
    stream.set_nonblocking(false).unwrap();
    stream.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
    let hello: (u32, String) = receive(&mut stream).unwrap();
    assert_eq!(hello, (PROTOCOL, nonce));
    send(&mut stream, &PluginRequest::Process { audio: vec![vec![0.0; 16]; 2], values: Default::default() }).unwrap();
    assert!(matches!(receive::<PluginResponse>(&mut stream).unwrap(), PluginResponse::Error(_)));
    send(&mut stream, &PluginRequest::Reset).unwrap();
    assert!(matches!(receive::<PluginResponse>(&mut stream).unwrap(), PluginResponse::Done));
    stream.write_all(&u32::MAX.to_le_bytes()).unwrap();
    assert!(receive::<PluginResponse>(&mut stream).is_err());
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        if let Some(status) = child.0.try_wait().unwrap() {
            assert!(!status.success());
            break;
        }
        assert!(Instant::now() < deadline, "helper did not reject oversized input");
        std::thread::sleep(Duration::from_millis(10));
    }
}
