//! Contract test: the REST reader against real Paperless-ngx 2.20 and 3.4
//!
//! Ignored by default. It pulls the Paperless image, so it runs only with
//! `FOLIO_PAPERLESS_IT=1 cargo test -p folio-infra --test paperless_contract -- --ignored`.

use std::collections::{BTreeMap, BTreeSet};
use std::time::{Duration, Instant};

use chrono::{NaiveDate, TimeZone, Utc};
use folio_core::ports::{
    digest_original, ChecksumAlgorithm, PaperlessDocument, PaperlessFieldType, PaperlessFieldValue,
    PaperlessReader, PaperlessRecord, PaperlessVersion,
};
use folio_infra::import::PaperlessHttpReader;
use futures::TryStreamExt;
use reqwest::multipart::{Form, Part};
use serde_json::{json, Value};
use testcontainers_modules::testcontainers::core::{IntoContainerPort, WaitFor};
use testcontainers_modules::testcontainers::runners::AsyncRunner;
use testcontainers_modules::testcontainers::{ContainerAsync, GenericImage, ImageExt};

const IMAGE: &str = "ghcr.io/paperless-ngx/paperless-ngx";
const ADMIN: &str = "admin";
const PASSWORD: &str = "contract-test-password";
const INVOICE: &[u8] = b"ACME Corp\nInvoice 42\nTotal EUR 123.45\n";
const LETTER: &[u8] = b"City Utilities\nWater bill for February\n";
const FLYER: &[u8] = b"Pizza flyer\n";
const SIGNED: &[u8] = b"City Utilities\nWater bill for February, signed\n";
const REDIS: &str = "7.4.12-alpine";

fn enabled() -> bool {
    std::env::var("FOLIO_PAPERLESS_IT").is_ok_and(|v| v == "1")
}

#[tokio::test]
#[ignore = "starts Paperless in Docker; set FOLIO_PAPERLESS_IT=1 and pass --ignored"]
async fn reads_paperless_2_20() {
    contract("2.20.15", ChecksumAlgorithm::Md5).await;
}

#[tokio::test]
#[ignore = "starts Paperless in Docker; set FOLIO_PAPERLESS_IT=1 and pass --ignored"]
async fn reads_paperless_3_4() {
    contract("3.4.2", ChecksumAlgorithm::Sha256).await;
}

struct Instance {
    _redis: ContainerAsync<GenericImage>,
    paperless: ContainerAsync<GenericImage>,
    url: String,
}

async fn start(tag: &str) -> Instance {
    let suffix = uuid::Uuid::new_v4().simple().to_string();
    let network = format!("folio-paperless-{suffix}");
    let redis_name = format!("folio-paperless-redis-{suffix}");
    let redis = GenericImage::new("redis", REDIS)
        .with_wait_for(WaitFor::message_on_stdout("Ready to accept connections"))
        .with_network(&network)
        .with_container_name(&redis_name)
        .start()
        .await
        .expect("Redis starts");
    let paperless = GenericImage::new(IMAGE, tag)
        .with_exposed_port(8000.tcp())
        .with_network(&network)
        .with_env_var("PAPERLESS_REDIS", format!("redis://{redis_name}:6379"))
        .with_env_var("PAPERLESS_SECRET_KEY", "contract-test-secret-key")
        .with_env_var("PAPERLESS_ADMIN_USER", ADMIN)
        .with_env_var("PAPERLESS_ADMIN_PASSWORD", PASSWORD)
        .with_env_var("PAPERLESS_TIME_ZONE", "Europe/Berlin")
        .with_startup_timeout(Duration::from_secs(600))
        .start()
        .await
        .expect("Paperless starts");
    let host = paperless.get_host().await.unwrap();
    let port = paperless.get_host_port_ipv4(8000).await.unwrap();
    let instance = Instance {
        _redis: redis,
        paperless,
        url: format!("http://{host}:{port}"),
    };
    instance.wait_until_ready().await;
    instance
}

impl Instance {
    async fn wait_until_ready(&self) {
        let client = reqwest::Client::new();
        let deadline = Instant::now() + Duration::from_secs(600);
        loop {
            let status = client
                .get(format!("{}/api/", self.url))
                .send()
                .await
                .map(|r| r.status().as_u16());
            // The admin is created after migrations, just before the server starts
            if status.as_ref().is_ok_and(|s| *s < 500) && self.token().await.is_some() {
                return;
            }
            if Instant::now() > deadline {
                let logs = self.paperless.stdout_to_vec().await.unwrap_or_default();
                panic!(
                    "Paperless did not start: {status:?}\n{}",
                    String::from_utf8_lossy(&logs)
                );
            }
            tokio::time::sleep(Duration::from_secs(2)).await;
        }
    }

    async fn token(&self) -> Option<String> {
        let response = reqwest::Client::new()
            .post(format!("{}/api/token/", self.url))
            .form(&[("username", ADMIN), ("password", PASSWORD)])
            .send()
            .await
            .ok()?;
        let body: Value = response.json().await.ok()?;
        body["token"].as_str().map(str::to_owned)
    }
}

/// Seeds data through the API the way a user would
struct Seeder {
    http: reqwest::Client,
    url: String,
    token: String,
}

impl Seeder {
    fn request(&self, method: reqwest::Method, path: &str) -> reqwest::RequestBuilder {
        self.http
            .request(method, format!("{}{path}", self.url))
            .header("Authorization", format!("Token {}", self.token))
            .header("Accept", "application/json; version=9")
    }

    async fn send(&self, request: reqwest::RequestBuilder) -> Value {
        let response = request.send().await.unwrap();
        let status = response.status();
        let url = response.url().clone();
        let body = response.text().await.unwrap();
        assert!(status.is_success(), "{url}: {status}: {body}");
        serde_json::from_str(&body).unwrap_or(Value::Null)
    }

    async fn post(&self, path: &str, body: Value) -> Value {
        self.send(self.request(reqwest::Method::POST, path).json(&body))
            .await
    }

    async fn create(&self, path: &str, body: Value) -> i64 {
        self.post(path, body).await["id"].as_i64().unwrap()
    }

    /// Uploads a document and waits for Paperless to consume it
    async fn upload(&self, name: &str, bytes: &[u8], fields: &[(&str, String)]) -> i64 {
        let mut form = Form::new().part(
            "document",
            Part::bytes(bytes.to_vec())
                .file_name(name.to_owned())
                .mime_str("text/plain")
                .unwrap(),
        );
        for (key, value) in fields {
            form = form.text(key.to_string(), value.clone());
        }
        let task = self
            .send(
                self.request(reqwest::Method::POST, "/api/documents/post_document/")
                    .multipart(form),
            )
            .await;
        self.consumed(task, name).await
    }

    /// Uploads a new file version of `root` (Paperless 3) and waits for it
    async fn add_version(&self, root: i64, name: &str, bytes: &[u8], label: &str) -> i64 {
        let form = Form::new()
            .part(
                "document",
                Part::bytes(bytes.to_vec())
                    .file_name(name.to_owned())
                    .mime_str("text/plain")
                    .unwrap(),
            )
            .text("version_label", label.to_owned());
        let task = self
            .send(
                self.request(
                    reqwest::Method::POST,
                    &format!("/api/documents/{root}/update_version/"),
                )
                .multipart(form),
            )
            .await;
        self.consumed(task, name).await
    }

    /// The document a consumption task created
    async fn consumed(&self, task: Value, name: &str) -> i64 {
        let task = task.as_str().unwrap().to_owned();
        let deadline = Instant::now() + Duration::from_secs(180);
        loop {
            let tasks = self
                .send(
                    self.request(reqwest::Method::GET, "/api/tasks/")
                        .query(&[("task_id", &task)]),
                )
                .await;
            let found = &tasks[0];
            match found["status"].as_str() {
                Some("SUCCESS") => {
                    let id = &found["related_document"];
                    return id
                        .as_i64()
                        .or_else(|| id.as_str().and_then(|s| s.parse().ok()))
                        .unwrap();
                }
                Some("FAILURE") => panic!("consuming {name} failed: {found}"),
                _ => {}
            }
            assert!(Instant::now() < deadline, "consuming {name} timed out");
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
    }
}

struct Seeded {
    inbox: i64,
    finance: i64,
    invoices: i64,
    acme: i64,
    invoice_type: i64,
    reference: i64,
    priority: i64,
    high: String,
    amount: i64,
    due: i64,
    related: i64,
    invoice: i64,
    letter: i64,
    flyer: i64,
    /// A file version of `letter`, on Paperless 3
    signed: Option<i64>,
}

async fn seed(instance: &Instance, versions: bool) -> Seeded {
    let s = Seeder {
        http: reqwest::Client::new(),
        url: instance.url.clone(),
        token: instance.token().await.unwrap(),
    };
    let inbox = s
        .create(
            "/api/tags/",
            json!({"name": "Inbox", "color": "#ff7f00", "is_inbox_tag": true}),
        )
        .await;
    let finance = s
        .create("/api/tags/", json!({"name": "Finance", "color": "#1f78b4"}))
        .await;
    let invoices = s
        .create(
            "/api/tags/",
            json!({"name": "Invoices", "color": "#e31a1c", "parent": finance}),
        )
        .await;
    let acme = s
        .create("/api/correspondents/", json!({"name": "ACME Corp"}))
        .await;
    let invoice_type = s
        .create("/api/document_types/", json!({"name": "Invoice"}))
        .await;
    let field = |name: &str, data_type: &str, extra: Value| json!({"name": name, "data_type": data_type, "extra_data": extra});
    let reference = s
        .create(
            "/api/custom_fields/",
            field("Reference", "string", json!({})),
        )
        .await;
    let priority_field = s
        .post(
            "/api/custom_fields/",
            field(
                "Priority",
                "select",
                json!({"select_options": [{"label": "High"}, {"label": "Low"}]}),
            ),
        )
        .await;
    let priority = priority_field["id"].as_i64().unwrap();
    let high = priority_field["extra_data"]["select_options"][0]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let amount = s
        .create(
            "/api/custom_fields/",
            field("Amount", "monetary", json!({"default_currency": "EUR"})),
        )
        .await;
    let due = s
        .create("/api/custom_fields/", field("Due date", "date", json!({})))
        .await;
    let related = s
        .create(
            "/api/custom_fields/",
            field("Related", "documentlink", json!({})),
        )
        .await;

    let invoice = s
        .upload(
            "invoice.txt",
            INVOICE,
            &[
                ("title", "Invoice 42".into()),
                ("correspondent", acme.to_string()),
                ("document_type", invoice_type.to_string()),
                ("tags", invoices.to_string()),
                ("created", "2024-01-15".into()),
                ("archive_serial_number", "1042".into()),
            ],
        )
        .await;
    let letter = s
        .upload("letter.txt", LETTER, &[("title", "Water bill".into())])
        .await;
    let flyer = s
        .upload("flyer.txt", FLYER, &[("title", "Old flyer".into())])
        .await;

    s.send(
        s.request(
            reqwest::Method::PATCH,
            &format!("/api/documents/{invoice}/"),
        )
        .json(&json!({"custom_fields": [
            {"field": reference, "value": "INV-42"},
            {"field": priority, "value": high},
            {"field": amount, "value": "EUR123.45"},
            {"field": due, "value": "2024-02-15"},
            {"field": related, "value": [letter]},
        ]})),
    )
    .await;
    s.post(
        &format!("/api/documents/{invoice}/notes/"),
        json!({"note": "Paid by bank transfer."}),
    )
    .await;
    s.send(s.request(reqwest::Method::DELETE, &format!("/api/documents/{flyer}/")))
        .await;
    let signed = match versions {
        true => Some(
            s.add_version(letter, "letter-signed.txt", SIGNED, "signed")
                .await,
        ),
        false => None,
    };

    Seeded {
        inbox,
        finance,
        invoices,
        acme,
        invoice_type,
        reference,
        priority,
        high,
        amount,
        due,
        related,
        invoice,
        letter,
        flyer,
        signed,
    }
}

async fn contract(version: &str, algorithm: ChecksumAlgorithm) {
    if !enabled() {
        eprintln!("SKIPPED: set FOLIO_PAPERLESS_IT=1 to run the Paperless contract test");
        return;
    }
    let _ = tracing_subscriber::fmt()
        .with_env_filter("folio_infra=debug")
        .with_test_writer()
        .try_init();
    let instance = start(version).await;
    // Paperless 3 keeps file versions and lists trashed documents' checksums
    let v3 = algorithm == ChecksumAlgorithm::Sha256;
    let seeded = seed(&instance, v3).await;
    let token = instance.token().await.unwrap();
    let reader = PaperlessHttpReader::new(&instance.url, &token).unwrap();

    assert_eq!(
        reader.source_info().await.unwrap().version,
        PaperlessVersion::parse(version).unwrap()
    );

    let records: Vec<PaperlessRecord> = reader.scan().await.unwrap().try_collect().await.unwrap();

    let mut tags = BTreeMap::new();
    let mut fields = BTreeMap::new();
    let mut named = BTreeSet::new();
    let mut documents = BTreeMap::new();
    let mut notes = Vec::new();
    let mut values = BTreeMap::new();
    for record in &records {
        match record {
            PaperlessRecord::Tag(t) => {
                tags.insert(t.id, t.clone());
            }
            PaperlessRecord::Correspondent(c) => {
                named.insert(("correspondent", c.id, c.name.clone()));
            }
            PaperlessRecord::DocumentType(t) => {
                named.insert(("type", t.id, t.name.clone()));
            }
            PaperlessRecord::CustomField(f) => {
                fields.insert(f.id, f.clone());
            }
            PaperlessRecord::Document(d) => {
                documents.insert(d.id, d.as_ref().clone());
            }
            PaperlessRecord::Note(n) => notes.push(n.clone()),
            PaperlessRecord::CustomFieldInstance(i) => {
                values.insert((i.document, i.field), i.value.clone());
            }
            PaperlessRecord::InvalidDocument(d) => panic!("invalid document {d:?}"),
        }
    }

    assert!(tags[&seeded.inbox].is_inbox_tag);
    assert_eq!(tags[&seeded.invoices].parent, Some(seeded.finance));
    assert_eq!(tags[&seeded.finance].parent, None);
    assert!(named.contains(&("correspondent", seeded.acme, "ACME Corp".into())));
    assert!(named.contains(&("type", seeded.invoice_type, "Invoice".into())));
    assert_eq!(
        fields[&seeded.amount].data_type,
        PaperlessFieldType::Monetary
    );
    assert_eq!(
        fields[&seeded.amount].default_currency.as_deref(),
        Some("EUR")
    );
    let options: Vec<_> = fields[&seeded.priority]
        .select_options
        .iter()
        .map(|o| (o.id.clone(), o.label.as_str()))
        .collect();
    assert_eq!(options[0], (Some(seeded.high.clone()), "High"));
    assert_eq!(options[1].1, "Low");

    let invoice = &documents[&seeded.invoice];
    assert_eq!(invoice.title, "Invoice 42");
    assert_eq!(invoice.correspondent, Some(seeded.acme));
    assert_eq!(invoice.document_type, Some(seeded.invoice_type));
    assert!(
        invoice.tags.contains(&seeded.invoices),
        "{:?}",
        invoice.tags
    );
    assert_eq!(invoice.archive_serial_number, Some(1042));
    assert_eq!(
        invoice.created,
        NaiveDate::from_ymd_opt(2024, 1, 15).unwrap()
    );
    assert_eq!(invoice.mime_type, "text/plain");
    assert!(
        invoice.content.contains("Invoice 42"),
        "{}",
        invoice.content
    );
    assert!(!invoice.trashed);
    assert!(invoice.added > Utc.with_ymd_and_hms(2025, 1, 1, 0, 0, 0).unwrap());

    let invoice_notes: Vec<_> = notes
        .iter()
        .filter(|n| n.document == seeded.invoice)
        .collect();
    assert_eq!(invoice_notes.len(), 1);
    assert_eq!(invoice_notes[0].body, "Paid by bank transfer.");
    assert!(!invoice_notes[0].deleted);

    let value = |field| values[&(seeded.invoice, field)].clone();
    assert_eq!(
        value(seeded.reference),
        Some(PaperlessFieldValue::Text("INV-42".into()))
    );
    assert_eq!(
        value(seeded.priority),
        Some(PaperlessFieldValue::Select(seeded.high.clone()))
    );
    assert_eq!(
        value(seeded.amount),
        Some(PaperlessFieldValue::Monetary("EUR123.45".into()))
    );
    assert_eq!(
        value(seeded.due),
        Some(PaperlessFieldValue::Date(
            NaiveDate::from_ymd_opt(2024, 2, 15).unwrap()
        ))
    );
    assert_eq!(
        value(seeded.related),
        Some(PaperlessFieldValue::DocumentLink(vec![seeded.letter]))
    );

    // Paperless 2 serves no checksum for a trashed document; its exports
    // leave trashed documents out too
    let mut files = vec![(seeded.invoice, INVOICE), (seeded.letter, LETTER)];
    if v3 {
        assert!(documents[&seeded.flyer].trashed);
        assert_eq!(documents[&seeded.flyer].size, None);
        files.push((seeded.flyer, FLYER));
    } else {
        assert!(!documents.contains_key(&seeded.flyer));
    }

    if let Some(signed) = seeded.signed {
        // The root keeps its own file and content; the version comes apart
        let letter = &documents[&seeded.letter];
        assert!(letter.content.contains("February"), "{}", letter.content);
        assert!(!letter.content.contains("signed"), "{}", letter.content);
        let version = &documents[&signed];
        let info = version.version_of.as_ref().unwrap();
        assert_eq!(info.root_document, seeded.letter);
        assert_eq!(info.index, Some(1));
        assert_eq!(info.label.as_deref(), Some("signed"));
        assert!(version.content.contains("signed"), "{}", version.content);
        assert_eq!(version.title, letter.title);
        files.push((signed, SIGNED));
    }

    for (id, bytes) in files {
        let document: &PaperlessDocument = &documents[&id];
        assert_eq!(document.checksum.algorithm, algorithm, "doc {id}");
        let expected = digest_original(bytes, algorithm).await.unwrap();
        assert!(expected.matches(&document.checksum), "doc {id}");
        if !document.trashed {
            assert_eq!(document.size, Some(bytes.len() as u64), "doc {id}");
        }

        let original = reader.open_original(document).await.unwrap();
        let read = digest_original(original.reader, algorithm).await.unwrap();
        assert!(read.matches(&document.checksum), "doc {id} download");
    }

    let ids = reader.list_document_ids().await.unwrap();
    assert_eq!(
        ids.ids().iter().copied().collect::<Vec<_>>(),
        [seeded.invoice, seeded.letter]
    );

    let later = Utc::now() + chrono::Duration::days(1);
    let recent: Vec<_> = reader
        .modified_since(later)
        .await
        .unwrap()
        .try_collect()
        .await
        .unwrap();
    assert!(
        !recent
            .iter()
            .any(|r| matches!(r, PaperlessRecord::Document(_))),
        "{recent:?}"
    );
}
