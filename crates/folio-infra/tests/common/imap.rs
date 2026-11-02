use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpListener;
use tokio::task::JoinHandle;

#[derive(Debug, PartialEq)]
pub struct MailboxState {
    pub imported_present: bool,
    pub imported_deleted: bool,
    pub unrelated_present: bool,
    pub unrelated_deleted: bool,
    pub destination_copies: usize,
}

pub struct ImapMailbox {
    pub port: u16,
    server: JoinHandle<Vec<MailboxState>>,
}

impl ImapMailbox {
    pub async fn start(capabilities: &'static str, rejected: Option<&'static str>) -> Self {
        Self::with_messages(capabilities, rejected, vec![(Some(1), "From: a@example.test\r\nSubject: Imported mail\r\nMessage-ID: <imported@example.test>\r\n\r\nDocument body\r\n".into())]).await
    }

    pub async fn with_messages(
        capabilities: &'static str,
        rejected: Option<&'static str>,
        messages: Vec<(Option<u32>, String)>,
    ) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = tokio::spawn(async move {
            let mut states = Vec::new();
            for (validity, raw) in messages {
                let (socket, _) = listener.accept().await.unwrap();
                let mut socket = BufReader::new(socket);
                socket
                    .get_mut()
                    .write_all(b"* OK isolated IMAP fixture\r\n")
                    .await
                    .unwrap();
                let mut state = MailboxState {
                    imported_present: true,
                    imported_deleted: false,
                    unrelated_present: true,
                    unrelated_deleted: true,
                    destination_copies: 0,
                };
                let mut line = String::new();
                while socket.read_line(&mut line).await.unwrap() != 0 {
                    let (tag, command) = line.trim_end().split_once(' ').unwrap();
                    let mut status = "OK";
                    let response = if rejected.is_some_and(|prefix| command.starts_with(prefix)) {
                        status = "NO";
                        String::new()
                    } else if command.starts_with("LOGIN ") {
                        String::new()
                    } else if command == "CAPABILITY" {
                        format!("* CAPABILITY IMAP4rev1 {capabilities}\r\n")
                    } else if command.starts_with("SELECT ") {
                        let validity = validity
                            .map(|v| format!("* OK [UIDVALIDITY {v}] valid\r\n"))
                            .unwrap_or_default();
                        format!("* FLAGS (\\Seen \\Deleted)\r\n* 2 EXISTS\r\n{validity}")
                    } else if command.starts_with("UID SEARCH ") {
                        "* SEARCH 7\r\n".to_string()
                    } else if command == "UID FETCH 7 BODY.PEEK[]" {
                        format!("* 1 FETCH (UID 7 BODY[] {{{}}}\r\n{raw})\r\n", raw.len())
                    } else if command == "UID STORE 7 +FLAGS (\\Deleted)" {
                        state.imported_deleted = true;
                        String::new()
                    } else if command.starts_with("UID COPY 7 ") {
                        state.destination_copies += 1;
                        String::new()
                    } else if command.starts_with("UID MOVE 7 ") && capabilities.contains("MOVE") {
                        state.imported_present = false;
                        state.destination_copies += 1;
                        "* 1 EXPUNGE\r\n".to_string()
                    } else if command == "UID EXPUNGE 7" && capabilities.contains("UIDPLUS") {
                        if state.imported_deleted {
                            state.imported_present = false;
                        }
                        "* 1 EXPUNGE\r\n".to_string()
                    } else if matches!(command, "EXPUNGE" | "CLOSE") {
                        state.imported_present &= !state.imported_deleted;
                        state.unrelated_present &= !state.unrelated_deleted;
                        "* 1 EXPUNGE\r\n* 1 EXPUNGE\r\n".to_string()
                    } else if command == "LOGOUT" {
                        "* BYE logging out\r\n".to_string()
                    } else {
                        status = "BAD";
                        String::new()
                    };
                    let reply = format!("{response}{tag} {status} completed\r\n");
                    socket.get_mut().write_all(reply.as_bytes()).await.unwrap();
                    if command == "LOGOUT" {
                        break;
                    }
                    line.clear();
                }
                states.push(state);
            }
            states
        });
        Self { port, server }
    }

    pub async fn state(self) -> MailboxState {
        self.states().await.pop().unwrap()
    }

    pub async fn states(self) -> Vec<MailboxState> {
        tokio::time::timeout(std::time::Duration::from_secs(5), self.server)
            .await
            .expect("IMAP fixture did not finish")
            .unwrap()
    }
}
