//! Instrumented repository calls export numeric span fields as integers.

use folio_core::models::Id;
use folio_infra::Database;
use opentelemetry::trace::TracerProvider as _;
use opentelemetry::Value;
use opentelemetry_sdk::trace::{InMemorySpanExporter, SdkTracerProvider};
use tracing_subscriber::layer::SubscriberExt;
use uuid::Uuid;

#[tokio::test]
async fn unsigned_arguments_are_exported_as_integers() {
    let exporter = InMemorySpanExporter::default();
    let provider = SdkTracerProvider::builder()
        .with_simple_exporter(exporter.clone())
        .build();
    let subscriber = tracing_subscriber::registry()
        .with(tracing_opentelemetry::layer().with_tracer(provider.tracer("folio-test")));
    let _default = tracing::subscriber::set_default(subscriber);

    let dir = std::env::temp_dir().join(format!("folio-telemetry-{}", Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    let database = Database::connect(&format!("sqlite:{}/test.db?mode=rwc", dir.display()))
        .await
        .unwrap();
    database.migrate().await.unwrap();
    let repos = database.repositories().await;
    let vault = repos.vaults.instance_default().await.unwrap();

    repos
        .workflows
        .list_runs(vault, Id::new_v4(), 10)
        .await
        .unwrap();

    let spans = exporter.get_finished_spans().unwrap();
    let list_runs = spans
        .iter()
        .find(|span| span.name == "list_runs")
        .expect("a list_runs span");
    assert_eq!(
        list_runs
            .attributes
            .iter()
            .find(|kv| kv.key.as_str() == "limit")
            .map(|kv| kv.value.clone()),
        Some(Value::I64(10))
    );
    let _ = std::fs::remove_dir_all(dir);
}
