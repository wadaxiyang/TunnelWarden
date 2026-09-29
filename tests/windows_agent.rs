//! Disposable external agent fixture. The russh public server owns the entire
//! agent protocol and signing; no fixture code is compiled into the application.
//! Inspected Oryxis engine/agent.rs at 16ad2af: discover a unique Pageant pipe,
//! without modifying the real OpenSSH service, user's keys, or agent settings.
use tokio::net::windows::named_pipe::{ClientOptions, ServerOptions};

pub struct AgentFixture {
    accept: tokio::task::JoinHandle<()>,
    serve: tokio::task::JoinHandle<()>,
}
impl AgentFixture {
    pub async fn start(private_key: &str) -> Self {
        let user = std::env::var("USERNAME").unwrap();
        let path = format!(r"\\.\pipe\pageant.{user}.{}", uuid::Uuid::new_v4());
        let first = ServerOptions::new()
            .first_pipe_instance(true)
            .create(&path)
            .unwrap();
        let (tx, rx) = tokio::sync::mpsc::channel(8);
        let accept_path = path.clone();
        let accept = tokio::spawn(async move {
            let mut pipe = first;
            loop {
                if pipe.connect().await.is_err() {
                    break;
                }
                let next = ServerOptions::new().create(&accept_path).unwrap();
                if tx.send(Ok(pipe)).await.is_err() {
                    break;
                }
                pipe = next;
            }
        });
        let serve = tokio::spawn(async move {
            russh::keys::agent::server::serve(tokio_stream::wrappers::ReceiverStream::new(rx), ())
                .await
                .unwrap();
        });
        let stream = ClientOptions::new().open(&path).unwrap();
        let mut client = russh::keys::agent::client::AgentClient::connect(stream);
        let key = russh::keys::decode_secret_key(private_key, None).unwrap();
        client.add_identity(&key, &[]).await.unwrap();
        Self { accept, serve }
    }
}
impl Drop for AgentFixture {
    fn drop(&mut self) {
        self.accept.abort();
        self.serve.abort();
    }
}
