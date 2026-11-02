//! IMAP mailboxes: imports attachments (and optionally whole messages) from new mail

use std::collections::BTreeSet;
use std::future::Future;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use async_imap::{types::Capability, Session};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use futures::TryStreamExt;
use mail_parser::MessageParser;
use sha2::{Digest, Sha256};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::TcpStream;
use tracing::{debug, warn};

use folio_core::error::{Error, Result};
use folio_core::models::{
    glob_match, DocumentSource, EmailConfig, EmailOrigin, Id, IngestItem, IngestItemKey,
    IngestOutcome, LedgerEntry, MailAfterImport, MailBody, MailSecurity, PollReport, VaultId,
};
use folio_core::ports::{IngestLedger, IngestSink, IngestionSource};

use super::mime::guess_mime;

const COMMAND_TIMEOUT: Duration = Duration::from_secs(20);

trait Io: AsyncRead + AsyncWrite + Unpin + Send + std::fmt::Debug {}
impl<T: AsyncRead + AsyncWrite + Unpin + Send + std::fmt::Debug> Io for T {}

struct ImapSession {
    inner: Session<Box<dyn Io>>,
    deadline: Deadline,
}

struct Deadline {
    duration: Duration,
    expired: AtomicBool,
}

impl Deadline {
    fn new(duration: Duration) -> Self {
        Self {
            duration,
            expired: AtomicBool::new(false),
        }
    }

    async fn run<T>(&self, context: &str, operation: impl Future<Output = Result<T>>) -> Result<T> {
        match tokio::time::timeout(self.duration, operation).await {
            Ok(result) => result,
            Err(_) => {
                self.expired.store(true, Ordering::Relaxed);
                Err(imap_err(context, "timed out"))
            }
        }
    }
}

fn imap_err(context: &str, e: impl std::fmt::Display) -> Error {
    Error::Storage(format!("IMAP {context}: {e}"))
}

pub struct EmailSource {
    id: Id,
    name: String,
    config: EmailConfig,
    ledger: Arc<dyn IngestLedger>,
    /// Where the ledger entries live; only polling needs it, not connection tests
    vault: Option<VaultId>,
    command_timeout: Duration,
}

/// What was found in one message
#[derive(Debug)]
pub struct ParsedMail {
    pub message_id: Option<String>,
    pub items: Vec<IngestItem>,
    part_ids: Vec<u32>,
}

/// Content types match case-insensitively, with `image/*` style wildcards
fn type_allowed(allowed: &[String], content_type: &str) -> bool {
    allowed.iter().any(|p| glob_match(p, content_type))
}

fn safe_name(raw: &str) -> String {
    let cleaned: String = raw
        .chars()
        .map(|c| {
            if c.is_control() || matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|') {
                '_'
            } else {
                c
            }
        })
        .collect();
    let trimmed = cleaned.trim().trim_matches('.');
    if trimmed.is_empty() {
        "untitled".to_string()
    } else {
        trimmed.chars().take(120).collect()
    }
}

/// Pull the importable pieces out of a raw RFC 822 message
pub fn parse_mail(
    raw: &[u8],
    source_name: &str,
    mailbox: &str,
    config: &EmailConfig,
) -> Option<ParsedMail> {
    let message = MessageParser::default().parse(raw)?;

    let from = message
        .from()
        .and_then(|a| a.first())
        .map(|m| match (m.name(), m.address()) {
            (Some(name), Some(addr)) => format!("{name} <{addr}>"),
            (None, Some(addr)) => addr.to_string(),
            (Some(name), None) => name.to_string(),
            _ => String::new(),
        })
        .unwrap_or_default();
    let to: Vec<String> = message
        .to()
        .map(|list| {
            list.mailboxes()
                .filter_map(|m| m.address().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();
    let subject = message.subject().unwrap_or_default().to_string();
    let date: Option<DateTime<Utc>> = message
        .date()
        .and_then(|d| DateTime::from_timestamp(d.to_timestamp(), 0));
    let message_id = message.message_id().map(str::to_string);

    let origin = |filename: Option<&str>, content_type: Option<&str>| EmailOrigin {
        account: source_name.to_string(),
        mailbox: mailbox.to_string(),
        message_id: message_id.clone().unwrap_or_default(),
        from: from.clone(),
        to: to.clone(),
        subject: subject.clone(),
        date,
        attachment_filename: filename.map(str::to_string),
        attachment_content_type: content_type.map(str::to_string),
    };

    let mut items = Vec::new();
    let mut part_ids = Vec::new();
    for (index, part) in message.attachments().enumerate() {
        // Signature logos and tracking pixels are inline images, not documents.
        let inline_image = part.content_disposition().is_some_and(|d| d.is_inline())
            && part.content_type().is_some_and(|t| t.ctype() == "image");
        let Some(name) = part.attachment_name() else {
            continue;
        };
        if inline_image {
            continue;
        }
        let declared = part
            .content_type()
            .map(|t| format!("{}/{}", t.ctype(), t.subtype().unwrap_or("octet-stream")))
            .unwrap_or_default();
        let content_type = if declared.is_empty() || declared == "application/octet-stream" {
            guess_mime(name).to_string()
        } else {
            declared
        };
        if !type_allowed(&config.allowed_content_types, &content_type) {
            debug!("Ignoring attachment {name} of type {content_type}");
            continue;
        }
        let filename = safe_name(name);
        part_ids.push(u32::try_from(index).ok()?.checked_add(1)?);
        items.push(IngestItem {
            data: part.decoded().into_owned(),
            source: DocumentSource::email(
                source_name,
                origin(Some(&filename), Some(&content_type)),
            ),
            mime_type: content_type,
            filename,
            tags: Vec::new(),
        });
    }

    if config.mail_body == MailBody::Eml {
        part_ids.push(0);
        let title = if subject.trim().is_empty() {
            "message".to_string()
        } else {
            safe_name(&subject)
        };
        items.push(IngestItem {
            filename: format!("{title}.eml"),
            mime_type: "message/rfc822".to_string(),
            data: raw.to_vec(),
            source: DocumentSource::email(source_name, origin(None, Some("message/rfc822"))),
            tags: Vec::new(),
        });
    }

    Some(ParsedMail {
        message_id,
        items,
        part_ids,
    })
}

impl EmailSource {
    pub fn new(
        id: Id,
        name: &str,
        config: EmailConfig,
        ledger: Arc<dyn IngestLedger>,
        vault: Option<VaultId>,
    ) -> Self {
        Self {
            id,
            name: name.to_string(),
            config,
            ledger,
            vault,
            command_timeout: COMMAND_TIMEOUT,
        }
    }

    /// Set the deadline for each IMAP operation, including connection setup and logout.
    pub fn with_command_timeout(mut self, timeout: Duration) -> Self {
        self.command_timeout = timeout;
        self
    }

    fn vault(&self) -> Result<VaultId> {
        self.vault
            .ok_or_else(|| Error::Validation(format!("Source '{}' has no vault", self.name)))
    }

    async fn connect(&self) -> Result<ImapSession> {
        let password = self.config.resolve_password()?;
        let addr = (self.config.host.as_str(), self.config.port);
        let deadline = Deadline::new(self.command_timeout);
        let tcp = deadline
            .run("connect", async {
                TcpStream::connect(addr)
                    .await
                    .map_err(|e| imap_err("connect", e))
            })
            .await?;

        let stream: Box<dyn Io> = match self.config.security {
            MailSecurity::Plain => Box::new(tcp),
            MailSecurity::Tls => {
                let connector = native_tls::TlsConnector::builder()
                    .danger_accept_invalid_certs(self.config.accept_invalid_certs)
                    .build()
                    .map_err(|e| imap_err("tls", e))?;
                let connector = tokio_native_tls::TlsConnector::from(connector);
                let tls = deadline
                    .run("tls handshake", async {
                        connector
                            .connect(&self.config.host, tcp)
                            .await
                            .map_err(|e| imap_err("tls handshake", e))
                    })
                    .await?;
                Box::new(tls)
            }
        };

        let mut client = async_imap::Client::new(stream);
        deadline
            .run("greeting", async {
                client
                    .read_response()
                    .await
                    .map_err(|e| imap_err("greeting", e))?
                    .ok_or_else(|| imap_err("greeting", "connection closed"))
            })
            .await?;
        let inner = deadline
            .run("login", async {
                client
                    .login(&self.config.username, &password)
                    .await
                    .map_err(|(e, _)| imap_err("login", e))
            })
            .await?;
        Ok(ImapSession { inner, deadline })
    }

    async fn search(
        &self,
        session: &mut ImapSession,
        mailbox: &str,
    ) -> Result<(Option<u32>, BTreeSet<u32>)> {
        let selected = session
            .deadline
            .run("select", async {
                session
                    .inner
                    .select(mailbox)
                    .await
                    .map_err(|e| imap_err("select", e))
            })
            .await?;
        let uids = session
            .deadline
            .run("search", async {
                session
                    .inner
                    .uid_search(&self.config.criteria)
                    .await
                    .map_err(|e| imap_err("search", e))
            })
            .await?;
        Ok((selected.uid_validity, uids.into_iter().collect()))
    }

    fn message_key(&self, message_id: &str) -> String {
        format!("msg:{}:{message_id}", self.id)
    }

    fn uid_key(&self, mailbox: &str, validity: u32, uid: u32) -> Result<String> {
        let identity = serde_json::to_vec(&(
            self.id,
            &self.config.host,
            self.config.port,
            &self.config.username,
            self.config.security,
            mailbox,
            validity,
            uid,
        ))
        .map_err(|e| imap_err("message identity", e))?;
        Ok(format!(
            "imap-uid:v1:{}",
            hex::encode(Sha256::digest(identity))
        ))
    }

    async fn fetch_raw(session: &mut ImapSession, uid: u32) -> Result<Option<Vec<u8>>> {
        let fetches: Vec<_> = session
            .deadline
            .run("fetch", async {
                session
                    .inner
                    .uid_fetch(uid.to_string(), "BODY.PEEK[]")
                    .await
                    .map_err(|e| imap_err("fetch", e))?
                    .try_collect()
                    .await
                    .map_err(|e| imap_err("fetch", e))
            })
            .await?;
        Ok(fetches.iter().find_map(|f| f.body()).map(<[u8]>::to_vec))
    }

    async fn finish_message(&self, session: &mut ImapSession, uid: u32) -> Result<()> {
        let uid = uid.to_string();
        if self.config.after_import == MailAfterImport::MarkRead {
            return store(session, &uid, "+FLAGS (\\Seen)").await;
        }

        let capabilities = session
            .deadline
            .run("capabilities", async {
                session
                    .inner
                    .capabilities()
                    .await
                    .map_err(|e| imap_err("capabilities", e))
            })
            .await?;
        let supports = |extension: &str| {
            capabilities.iter().any(|capability| {
                matches!(capability, Capability::Atom(name) if name.eq_ignore_ascii_case(extension))
            })
        };
        if self.config.after_import == MailAfterImport::Move && supports("MOVE") {
            return session
                .deadline
                .run("UID move", async {
                    session
                        .inner
                        .uid_mv(&uid, self.config.move_to.as_deref().unwrap_or_default())
                        .await
                        .map_err(|e| imap_err("UID move", e))
                })
                .await;
        }
        if !supports("UIDPLUS") {
            return Err(imap_err(
                "cleanup",
                "server lacks UIDPLUS for safe deletion; leaving message unchanged",
            ));
        }
        if self.config.after_import == MailAfterImport::Move {
            session
                .deadline
                .run("copy", async {
                    session
                        .inner
                        .uid_copy(&uid, self.config.move_to.as_deref().unwrap_or_default())
                        .await
                        .map_err(|e| imap_err("copy", e))
                })
                .await?;
        }
        self.delete(session, &uid).await
    }

    async fn delete(&self, session: &mut ImapSession, uid: &str) -> Result<()> {
        store(session, uid, "+FLAGS (\\Deleted)").await?;
        session
            .deadline
            .run("UID expunge", async {
                session
                    .inner
                    .uid_expunge(uid)
                    .await
                    .map_err(|e| imap_err("UID expunge", e))?
                    .try_collect::<Vec<_>>()
                    .await
                    .map_err(|e| imap_err("UID expunge", e))?;
                Ok(())
            })
            .await?;
        Ok(())
    }

    async fn process_mailbox(
        &self,
        session: &mut ImapSession,
        mailbox: &str,
        sink: &dyn IngestSink,
        report: &mut PollReport,
    ) -> Result<()> {
        let (validity, uids) = self.search(session, mailbox).await?;
        for uid in uids {
            let Some(raw) = Self::fetch_raw(session, uid).await? else {
                continue;
            };
            let Some(mail) = parse_mail(&raw, &self.name, mailbox, &self.config) else {
                warn!("Message {uid} in {mailbox} is not parseable, leaving it alone");
                report.failed += 1;
                continue;
            };
            let key = match mail.message_id.as_deref() {
                Some(message_id) => self.message_key(message_id),
                None => {
                    let Some(validity) = validity.filter(|v| *v != 0) else {
                        warn!("Message {uid} in {mailbox} has no Message-ID or usable UIDVALIDITY, leaving it alone");
                        report.failed += 1;
                        continue;
                    };
                    self.uid_key(mailbox, validity, uid)?
                }
            };

            let already_done = self.ledger.find(self.vault()?, &key).await?.is_some();
            if !already_done {
                let mut all_ok = true;
                for (item, part_id) in mail.items.into_iter().zip(mail.part_ids) {
                    let item_key = match validity.filter(|v| *v != 0) {
                        Some(validity) => IngestItemKey::email(mailbox, validity, uid, part_id)?,
                        None => IngestItemKey::email_message(
                            mailbox,
                            mail.message_id.as_deref().unwrap_or_default(),
                            part_id,
                        )?,
                    };
                    match sink.ingest_observed(self.id, item_key, item).await {
                        Ok(IngestOutcome::Imported(_)) => report.imported += 1,
                        Ok(IngestOutcome::Busy) => {
                            all_ok = false;
                            report.recheck_in = Some(Duration::from_secs(2));
                        }
                        Ok(_) => report.skipped += 1,
                        Err(e) => {
                            warn!("Importing from message {uid} failed: {e}");
                            report.failed += 1;
                            all_ok = false;
                        }
                    }
                }
                if !all_ok {
                    // Leave the mail untouched; the content hash keeps the retry from duplicating.
                    continue;
                }
                self.ledger
                    .insert(
                        self.vault()?,
                        &LedgerEntry {
                            vault_id: self.vault()?,
                            key: key.clone(),
                            source_id: self.id,
                            document_id: None,
                            outcome: "processed".to_string(),
                            created_at: Utc::now(),
                        },
                    )
                    .await?;
            }
            // Also runs for mails recorded earlier whose clean-up failed back then.
            if let Err(e) = self.finish_message(session, uid).await {
                warn!("Could not clean up message {uid}: {e}");
                report.failed += 1;
                if session.deadline.expired.load(Ordering::Relaxed) {
                    return Ok(());
                }
            }
        }
        Ok(())
    }
}

async fn store(session: &mut ImapSession, uid: &str, query: &str) -> Result<()> {
    session
        .deadline
        .run("store", async {
            session
                .inner
                .uid_store(uid, query)
                .await
                .map_err(|e| imap_err("store", e))?
                .try_collect::<Vec<_>>()
                .await
                .map_err(|e| imap_err("store", e))?;
            Ok(())
        })
        .await
}

async fn logout(session: &mut ImapSession) -> Result<()> {
    session
        .deadline
        .run("logout", async {
            session
                .inner
                .logout()
                .await
                .map_err(|e| imap_err("logout", e))
        })
        .await
}

#[async_trait]
impl IngestionSource for EmailSource {
    async fn poll(&self, sink: &dyn IngestSink) -> Result<PollReport> {
        let mut session = self.connect().await?;
        let mut report = PollReport::default();
        let mut first_error = None;
        for mailbox in &self.config.mailboxes {
            if let Err(e) = self
                .process_mailbox(&mut session, mailbox, sink, &mut report)
                .await
            {
                warn!("Mailbox {mailbox} of '{}' failed: {e}", self.name);
                first_error.get_or_insert(e);
            }
            if session.deadline.expired.load(Ordering::Relaxed) {
                return match first_error {
                    Some(e) => Err(e),
                    None => Ok(report),
                };
            }
        }
        if let Err(e) = logout(&mut session).await {
            first_error.get_or_insert(e);
        }
        match first_error {
            Some(e) => Err(e),
            None => Ok(report),
        }
    }

    async fn test(&self) -> Result<String> {
        let mut session = self.connect().await?;
        let mut parts = Vec::new();
        for mailbox in &self.config.mailboxes {
            let (_, uids) = self.search(&mut session, mailbox).await?;
            let count = uids.len();
            parts.push(format!("{count} matching in {mailbox}"));
        }
        logout(&mut session).await?;
        Ok(format!(
            "Connected as {}: {}",
            self.config.username,
            parts.join(", ")
        ))
    }

    fn interval_secs(&self) -> u64 {
        self.config.poll_interval_secs
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(body: MailBody) -> EmailConfig {
        serde_json::from_value(serde_json::json!({
            "host": "h", "username": "u", "password": "p",
            "mail_body": body,
        }))
        .unwrap()
    }

    const MAIL: &str = "From: Billing <billing@shop.test>\r\n\
To: me@home.test, other@home.test\r\n\
Subject: Your invoice\r\n\
Message-ID: <abc@shop.test>\r\n\
Date: Fri, 21 Nov 1997 09:55:06 -0600\r\n\
MIME-Version: 1.0\r\n\
Content-Type: multipart/mixed; boundary=\"b\"\r\n\r\n\
--b\r\nContent-Type: text/plain\r\n\r\nSee attached.\r\n\
--b\r\nContent-Type: application/pdf; name=\"inv.pdf\"\r\n\
Content-Disposition: attachment; filename=\"inv.pdf\"\r\n\
Content-Transfer-Encoding: base64\r\n\r\nJVBERi0xLjQK\r\n\
--b\r\nContent-Type: application/zip; name=\"x.zip\"\r\n\
Content-Disposition: attachment; filename=\"x.zip\"\r\n\
Content-Transfer-Encoding: base64\r\n\r\nUEsDBA==\r\n\
--b\r\nContent-Type: image/png; name=\"logo.png\"\r\n\
Content-Disposition: inline; filename=\"logo.png\"\r\nContent-ID: <logo>\r\n\
Content-Transfer-Encoding: base64\r\n\r\niVBORw0K\r\n\
--b--\r\n";

    #[test]
    fn attachment_identity_does_not_shift_when_allowed_types_change() {
        let mut only_zip = config(MailBody::None);
        only_zip.allowed_content_types = vec!["application/zip".into()];
        let mut all_types = only_zip.clone();
        all_types.allowed_content_types = vec!["*/*".into()];
        let zip = parse_mail(MAIL.as_bytes(), "work", "INBOX", &only_zip).unwrap();
        let all = parse_mail(MAIL.as_bytes(), "work", "INBOX", &all_types).unwrap();
        assert_eq!(zip.items[0].filename, "x.zip");
        assert_eq!(all.items[1].filename, "x.zip");
        assert_eq!(zip.part_ids[0], all.part_ids[1]);
        assert_ne!(all.part_ids[0], all.part_ids[1]);
    }

    #[test]
    fn extracts_allowed_attachments_with_origin() {
        let mail = parse_mail(MAIL.as_bytes(), "work", "INBOX", &config(MailBody::None)).unwrap();
        assert_eq!(mail.message_id.as_deref(), Some("abc@shop.test"));
        assert_eq!(mail.items.len(), 1, "zip and inline logo are ignored");
        let item = &mail.items[0];
        assert_eq!(item.filename, "inv.pdf");
        assert_eq!(item.mime_type, "application/pdf");
        assert_eq!(item.data, b"%PDF-1.4\n");
        let email = item.source.email.as_ref().unwrap();
        assert_eq!(email.from, "Billing <billing@shop.test>");
        assert_eq!(email.to, ["me@home.test", "other@home.test"]);
        assert_eq!(email.subject, "Your invoice");
        assert_eq!(email.attachment_filename.as_deref(), Some("inv.pdf"));
        assert!(email.date.is_some());
    }

    #[test]
    fn mail_body_adds_the_message_as_eml() {
        let mail = parse_mail(MAIL.as_bytes(), "work", "INBOX", &config(MailBody::Eml)).unwrap();
        let eml = mail.items.last().unwrap();
        assert_eq!(eml.filename, "Your invoice.eml");
        assert_eq!(eml.mime_type, "message/rfc822");
        assert_eq!(eml.data, MAIL.as_bytes());
    }

    #[test]
    fn unsafe_filenames_are_cleaned() {
        assert_eq!(safe_name("../etc/passwd"), "_etc_passwd");
        assert_eq!(safe_name("  "), "untitled");
    }
}
