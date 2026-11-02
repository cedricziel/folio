//! IMAP ingestion against isolated loopback mailboxes and a GreenMail container.
//!
//! GreenMail tests need Docker and skip without it unless `FOLIO_REQUIRE_DOCKER` is set.

mod common;
#[path = "common/imap.rs"]
mod imap;

use std::sync::{Arc, Mutex};

use async_imap::Session;
use async_trait::async_trait;
use common::{container_or_skip, ingest::Harness};
use folio_core::error::Result;
use folio_core::models::{
    Document, EmailConfig, Id, IngestItem, IngestOutcome, MailAfterImport, MailBody, MailSecurity,
    SourceConditions, SourceKind, TextMatch, VaultId,
};
use folio_core::ports::{IngestSink, IngestionSource};
use folio_infra::EmailSource;
use futures::TryStreamExt;
use testcontainers_modules::testcontainers::{
    core::{IntoContainerPort, WaitFor},
    runners::AsyncRunner,
    ContainerAsync, GenericImage, ImageExt,
};
use tokio::net::TcpStream;

type Imap = Session<TcpStream>;

struct GreenMail {
    _container: ContainerAsync<GenericImage>,
    host: String,
    imap_port: u16,
    imaps_port: u16,
}

impl GreenMail {
    async fn start() -> Option<Self> {
        let started = GenericImage::new("greenmail/standalone", "2.1.4")
            .with_exposed_port(3143.tcp())
            .with_exposed_port(3993.tcp())
            .with_wait_for(WaitFor::message_on_stdout("Starting GreenMail API server"))
            .with_env_var("JAVA_OPTS", "-Djava.net.preferIPv4Stack=true -Xmx256m")
            .start()
            .await;
        let container = container_or_skip("GreenMail", started)?;
        Some(Self {
            host: container.get_host().await.unwrap().to_string(),
            imap_port: container.get_host_port_ipv4(3143.tcp()).await.unwrap(),
            imaps_port: container.get_host_port_ipv4(3993.tcp()).await.unwrap(),
            _container: container,
        })
    }

    async fn login(&self, user: &str) -> Imap {
        let tcp = TcpStream::connect((self.host.as_str(), self.imap_port))
            .await
            .unwrap();
        let mut client = async_imap::Client::new(tcp);
        client.read_response().await.unwrap();
        client
            .login(user, "secret")
            .await
            .map_err(|(e, _)| e)
            .unwrap()
    }

    async fn seed(&self, user: &str, messages: &[String]) {
        let mut session = self.login(user).await;
        for message in messages {
            session
                .append("INBOX", None, None, message.as_bytes())
                .await
                .unwrap();
        }
        session.logout().await.unwrap();
    }

    fn config(&self, user: &str) -> EmailConfig {
        serde_json::from_value(serde_json::json!({
            "host": self.host,
            "port": self.imap_port,
            "security": MailSecurity::Plain,
            "username": user,
            "password": "secret",
        }))
        .unwrap()
    }

    fn source(&self, config: EmailConfig, h: &Harness) -> EmailSource {
        EmailSource::new(h.source_id, "work", config, h.ledger.clone(), Some(h.vault))
    }

    /// (seen, deleted) flags per UID in a mailbox
    async fn flags(&self, user: &str, mailbox: &str) -> Vec<Vec<String>> {
        let mut session = self.login(user).await;
        if session.select(mailbox).await.is_err() {
            return Vec::new();
        }
        let fetched: Vec<_> = session
            .fetch("1:*", "FLAGS")
            .await
            .map(|s| s.try_collect::<Vec<_>>())
            .unwrap()
            .await
            .unwrap_or_default();
        fetched
            .iter()
            .map(|f| f.flags().map(|flag| format!("{flag:?}")).collect())
            .collect()
    }
}

fn mail(id: &str, from: &str, subject: &str, attachments: &[(&str, &str, &str)]) -> String {
    let mut raw = format!(
        "From: {from}\r\nTo: me@home.test\r\nSubject: {subject}\r\nMessage-ID: <{id}@test>\r\n\
Date: Fri, 21 Nov 2025 09:55:06 +0000\r\nMIME-Version: 1.0\r\n\
Content-Type: multipart/mixed; boundary=\"b\"\r\n\r\n\
--b\r\nContent-Type: text/plain\r\n\r\nHello\r\n"
    );
    for (name, content_type, base64) in attachments {
        raw.push_str(&format!(
            "--b\r\nContent-Type: {content_type}; name=\"{name}\"\r\n\
Content-Disposition: attachment; filename=\"{name}\"\r\n\
Content-Transfer-Encoding: base64\r\n\r\n{base64}\r\n"
        ));
    }
    raw.push_str("--b--\r\n");
    raw
}

const PDF: &str = "JVBERi0xLjQK";
const ZIP: &str = "UEsDBA==";

#[derive(Default)]
struct Collect(Mutex<Vec<IngestItem>>);

#[async_trait]
impl IngestSink for Collect {
    async fn ingest(&self, _s: Id, item: IngestItem) -> Result<IngestOutcome> {
        self.0.lock().unwrap().push(item);
        Ok(IngestOutcome::Imported(Box::new(Document::new(
            VaultId::new(),
            "t",
        ))))
    }
}

#[tokio::test]
async fn imports_allowed_attachments_once_and_marks_the_mail_read() {
    let Some(gm) = GreenMail::start().await else {
        return;
    };
    gm.seed(
        "alice",
        &[
            mail(
                "1",
                "Billing <billing@shop.test>",
                "Invoice",
                &[
                    ("inv.pdf", "application/pdf", PDF),
                    ("x.zip", "application/zip", ZIP),
                ],
            ),
            mail("2", "friend@x.test", "Hi", &[]),
        ],
    )
    .await;
    let h = Harness::new().await;
    let source = gm.source(gm.config("alice"), &h);
    let sink = Collect::default();

    let first = source.poll(&sink).await.unwrap();
    assert_eq!(first.imported, 1);
    {
        let items = sink.0.lock().unwrap();
        assert_eq!(items[0].filename, "inv.pdf");
        let email = items[0].source.email.as_ref().unwrap();
        assert_eq!(email.from, "Billing <billing@shop.test>");
        assert_eq!(email.subject, "Invoice");
        assert_eq!(email.mailbox, "INBOX");
        assert_eq!(email.message_id, "1@test");
        assert_eq!(email.to, ["me@home.test"]);
    }

    assert_eq!(source.poll(&sink).await.unwrap().imported, 0, "now read");
    assert!(gm
        .flags("alice", "INBOX")
        .await
        .iter()
        .all(|f| f.iter().any(|f| f.contains("Seen"))));

    // Even if the mail turns unread again, the processed message-id table stops a re-import.
    let mut session = gm.login("alice").await;
    session.select("INBOX").await.unwrap();
    session
        .store("1:*", "-FLAGS (\\Seen)")
        .await
        .unwrap()
        .try_collect::<Vec<_>>()
        .await
        .unwrap();
    session.logout().await.unwrap();
    assert_eq!(source.poll(&sink).await.unwrap().imported, 0);
    assert_eq!(sink.0.lock().unwrap().len(), 1);
}

async fn seed_cleanup_mailbox(gm: &GreenMail, user: &str) {
    gm.seed(
        user,
        &[
            mail(
                "imported",
                "a@x.test",
                "Import me",
                &[("a.pdf", "application/pdf", PDF)],
            ),
            mail("unrelated", "b@x.test", "Keep me", &[]),
        ],
    )
    .await;
    let mut session = gm.login(user).await;
    session.select("INBOX").await.unwrap();
    session
        .uid_store("2", "+FLAGS (\\Seen \\Deleted)")
        .await
        .unwrap()
        .try_collect::<Vec<_>>()
        .await
        .unwrap();
    session.logout().await.unwrap();
}

async fn assert_unrelated_mail_remains(gm: &GreenMail, user: &str) {
    let mut session = gm.login(user).await;
    session.select("INBOX").await.unwrap();
    let uids = session.uid_search("ALL").await.unwrap();
    assert_eq!(
        uids,
        [2].into_iter().collect(),
        "only unrelated UID 2 remains"
    );
    let deleted = session.uid_search("DELETED").await.unwrap();
    assert_eq!(
        deleted, uids,
        "the unrelated message keeps its deleted flag"
    );
    session.logout().await.unwrap();
}

#[tokio::test]
async fn move_preserves_unrelated_pre_deleted_mail() {
    let Some(gm) = GreenMail::start().await else {
        return;
    };
    seed_cleanup_mailbox(&gm, "mover").await;
    let mut session = gm.login("mover").await;
    session.create("Processed").await.unwrap();
    session.logout().await.unwrap();
    let h = Harness::new().await;
    let mut config = gm.config("mover");
    config.after_import = MailAfterImport::Move;
    config.move_to = Some("Processed".into());
    let sink = Collect::default();

    let report = gm.source(config, &h).poll(&sink).await.unwrap();

    assert_eq!(report.imported, 1);
    assert_unrelated_mail_remains(&gm, "mover").await;
    assert_eq!(gm.flags("mover", "Processed").await.len(), 1);
}

#[tokio::test]
async fn delete_preserves_unrelated_pre_deleted_mail() {
    let Some(gm) = GreenMail::start().await else {
        return;
    };
    seed_cleanup_mailbox(&gm, "deleter").await;
    let h = Harness::new().await;
    let mut config = gm.config("deleter");
    config.after_import = MailAfterImport::Delete;
    let sink = Collect::default();

    let report = gm.source(config, &h).poll(&sink).await.unwrap();

    assert_eq!(report.imported, 1);
    assert_unrelated_mail_remains(&gm, "deleter").await;
}

#[tokio::test]
async fn pre_ingest_rules_discard_newsletters_and_the_body_can_be_kept_as_eml() {
    let Some(gm) = GreenMail::start().await else {
        return;
    };
    let h = Harness::new().await;
    h.rule(
        "Ignore newsletters",
        SourceConditions {
            kind: Some(SourceKind::Email),
            email_from: Some(TextMatch::Contains("newsletter".into())),
            ..Default::default()
        },
    )
    .await;
    let pdf = [("a.pdf", "application/pdf", PDF)];
    gm.seed(
        "bob",
        &[
            mail("n1", "News <newsletter@spam.test>", "Deals", &pdf),
            mail("w1", "Bank <bank@x.test>", "Statement", &pdf),
        ],
    )
    .await;
    let mut config = gm.config("bob");
    config.mail_body = MailBody::Eml;

    struct Pipeline(
        Arc<folio_core::services::IngestPipeline>,
        folio_core::models::VaultId,
    );
    #[async_trait]
    impl IngestSink for Pipeline {
        async fn ingest(&self, s: Id, item: IngestItem) -> Result<IngestOutcome> {
            folio_core::ports::VaultIngest::ingest(self.0.as_ref(), self.1, s, item).await
        }
    }
    let report = gm
        .source(config, &h)
        .poll(&Pipeline(h.pipeline.clone(), h.vault))
        .await
        .unwrap();

    // Newsletter: attachment and body skipped. Bank: attachment and .eml imported.
    assert_eq!((report.imported, report.skipped), (2, 2));
    let mut names: Vec<_> = h
        .documents
        .list_all()
        .await
        .unwrap()
        .into_iter()
        .map(|d| d.original_filename.unwrap())
        .collect();
    names.sort();
    assert_eq!(names, ["Statement.eml", "a.pdf"]);
}

#[tokio::test]
async fn tls_connections_and_failed_logins_are_reported() {
    let Some(gm) = GreenMail::start().await else {
        return;
    };
    let h = Harness::new().await;
    let mut config = gm.config("carol");
    config.security = MailSecurity::Tls;
    config.port = gm.imaps_port;

    let strict = gm.source(config.clone(), &h);
    assert!(strict.test().await.is_err(), "self-signed cert is rejected");

    config.accept_invalid_certs = true;
    let summary = gm.source(config, &h).test().await.unwrap();
    assert!(summary.contains("0 matching in INBOX"), "{summary}");

    let mut unreachable = gm.config("carol");
    unreachable.port = 1;
    assert!(gm.source(unreachable, &h).test().await.is_err());
}

async fn cleanup_state(
    capabilities: &'static str,
    action: MailAfterImport,
    rejected: Option<&'static str>,
) -> imap::MailboxState {
    let mailbox = imap::ImapMailbox::start(capabilities, rejected).await;
    let h = Harness::new().await;
    let config = serde_json::from_value(serde_json::json!({
        "host": "127.0.0.1", "port": mailbox.port, "security": "plain",
        "username": "fixture", "password": "fixture", "mail_body": "eml",
        "after_import": action, "move_to": "Processed",
    }))
    .unwrap();
    let source = EmailSource::new(
        h.source_id,
        "fixture",
        config,
        h.ledger.clone(),
        Some(h.vault),
    );
    let sink = Collect::default();
    let report = tokio::time::timeout(std::time::Duration::from_secs(5), source.poll(&sink))
        .await
        .expect("IMAP poll did not finish")
        .unwrap();
    assert_eq!(report.imported, 1);
    assert_eq!(sink.0.lock().unwrap().len(), 1);
    mailbox.state().await
}

#[tokio::test]
async fn uidplus_delete_removes_only_the_imported_uid() {
    let state = cleanup_state("UIDPLUS", MailAfterImport::Delete, None).await;
    assert!(!state.imported_present);
    assert!(state.unrelated_present && state.unrelated_deleted);
    assert_eq!(state.destination_copies, 0);
}

#[tokio::test]
async fn move_extension_works_without_uidplus() {
    let state = cleanup_state("MOVE", MailAfterImport::Move, None).await;
    assert!(!state.imported_present);
    assert!(state.unrelated_present && state.unrelated_deleted);
    assert_eq!(state.destination_copies, 1);
}

#[tokio::test]
async fn uidplus_move_fallback_preserves_unrelated_deleted_mail() {
    let state = cleanup_state("UIDPLUS", MailAfterImport::Move, None).await;
    assert!(!state.imported_present);
    assert!(state.unrelated_present && state.unrelated_deleted);
    assert_eq!(state.destination_copies, 1);
}

fn unchanged_mailbox() -> imap::MailboxState {
    imap::MailboxState {
        imported_present: true,
        imported_deleted: false,
        unrelated_present: true,
        unrelated_deleted: true,
        destination_copies: 0,
    }
}

#[tokio::test]
async fn delete_without_uidplus_leaves_source_flags_unchanged() {
    assert_eq!(
        cleanup_state("MOVE", MailAfterImport::Delete, None).await,
        unchanged_mailbox()
    );
}

#[tokio::test]
async fn move_without_move_or_uidplus_leaves_both_mailboxes_unchanged() {
    assert_eq!(
        cleanup_state("", MailAfterImport::Move, None).await,
        unchanged_mailbox()
    );
}

#[tokio::test]
async fn rejected_uid_expunge_never_falls_back_to_mailbox_expunge() {
    let state = cleanup_state("UIDPLUS", MailAfterImport::Delete, Some("UID EXPUNGE")).await;
    assert!(state.imported_present && state.imported_deleted);
    assert!(state.unrelated_present && state.unrelated_deleted);
}

#[tokio::test]
async fn rejected_uid_move_never_falls_back_to_copy_or_expunge() {
    assert_eq!(
        cleanup_state("MOVE UIDPLUS", MailAfterImport::Move, Some("UID MOVE")).await,
        unchanged_mailbox()
    );
}
