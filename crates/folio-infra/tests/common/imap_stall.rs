use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpListener;
use tokio::sync::oneshot;
use tokio::task::JoinHandle;

pub struct StallingMailbox {
    pub port: u16,
    pub released: oneshot::Receiver<()>,
    server: JoinHandle<()>,
}

impl StallingMailbox {
    pub async fn start(phase: &'static str) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let (released, receiver) = oneshot::channel();
        let server = tokio::spawn(async move {
            let (socket, _) = listener.accept().await.unwrap();
            let mut socket = BufReader::new(socket);
            if matches!(phase, "greeting" | "tls handshake") {
                let mut pending = Vec::new();
                socket.read_to_end(&mut pending).await.unwrap();
            } else {
                socket
                    .get_mut()
                    .write_all(b"* OK isolated stall fixture\r\n")
                    .await
                    .unwrap();
                loop {
                    let mut line = String::new();
                    if socket.read_line(&mut line).await.unwrap() == 0 {
                        panic!("connection closed before reaching {phase}");
                    }
                    let (tag, command) = line.trim_end().split_once(' ').unwrap();
                    if phase == "first error" && command.starts_with("SELECT ") {
                        socket
                            .get_mut()
                            .write_all(format!("{tag} NO isolated-select-failure\r\n").as_bytes())
                            .await
                            .unwrap();
                        continue;
                    }
                    if command.starts_with(phase) || phase == "first error" && command == "LOGOUT" {
                        if phase == "UID FETCH" {
                            socket
                                .get_mut()
                                .write_all(b"* 1 FETCH (UID 7 BODY[] {1000}\r\npartial")
                                .await
                                .unwrap();
                        } else if phase == "UID STORE" {
                            socket
                                .get_mut()
                                .write_all(b"* 1 FETCH (UID 7 FLAGS (\\Seen))\r\n")
                                .await
                                .unwrap();
                        } else if phase == "UID EXPUNGE" {
                            socket
                                .get_mut()
                                .write_all(b"* 1 EXPUNGE\r\n")
                                .await
                                .unwrap();
                        }
                        let mut pending = Vec::new();
                        socket.read_to_end(&mut pending).await.unwrap();
                        assert!(pending.is_empty(), "timed out connection was reused");
                        break;
                    }
                    reply(&mut socket, tag, command, phase == "UID MOVE").await;
                }
            }
            drop(socket);
            released.send(()).unwrap();
            if phase != "tls handshake" {
                let (socket, _) = listener.accept().await.unwrap();
                let mut socket = BufReader::new(socket);
                socket
                    .get_mut()
                    .write_all(b"* OK recovered fixture\r\n")
                    .await
                    .unwrap();
                loop {
                    let mut line = String::new();
                    if socket.read_line(&mut line).await.unwrap() == 0 {
                        break;
                    }
                    let (tag, command) = line.trim_end().split_once(' ').unwrap();
                    reply(&mut socket, tag, command, phase == "UID MOVE").await;
                    if command == "LOGOUT" {
                        break;
                    }
                }
            }
        });
        Self {
            port,
            released: receiver,
            server,
        }
    }

    pub async fn finish(self) {
        self.server.await.unwrap();
    }
}

async fn reply(
    socket: &mut BufReader<tokio::net::TcpStream>,
    tag: &str,
    command: &str,
    supports_move: bool,
) {
    let raw = "From: a@example.test\r\nMessage-ID: <deadline@example.test>\r\n\r\nDocument\r\n";
    let response = if command.starts_with("LOGIN ")
        || command.starts_with("UID ")
            && !command.starts_with("UID FETCH")
            && !command.starts_with("UID SEARCH")
    {
        String::new()
    } else if command.starts_with("SELECT ") {
        "* 1 EXISTS\r\n* OK [UIDVALIDITY 1] valid\r\n".into()
    } else if command.starts_with("UID SEARCH ") {
        "* SEARCH 7\r\n".into()
    } else if command.starts_with("UID FETCH ") {
        format!("* 1 FETCH (UID 7 BODY[] {{{}}}\r\n{raw})\r\n", raw.len())
    } else if command == "CAPABILITY" {
        if supports_move {
            "* CAPABILITY IMAP4rev1 UIDPLUS MOVE\r\n".into()
        } else {
            "* CAPABILITY IMAP4rev1 UIDPLUS\r\n".into()
        }
    } else if command == "LOGOUT" {
        "* BYE done\r\n".into()
    } else {
        panic!("unexpected fixture command {command}");
    };
    socket
        .get_mut()
        .write_all(format!("{response}{tag} OK done\r\n").as_bytes())
        .await
        .unwrap();
}
