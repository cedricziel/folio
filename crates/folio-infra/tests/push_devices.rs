mod common;

use chrono::{Duration, Utc};
use folio_auth::{AccessToken, AuthStore, OAuthClient, RefreshToken, User};
use folio_core::models::{
    CreateDocument, DevicePlatform, DeviceRegistration, DocumentOutcome, NotificationKind,
    NotificationPreferences, PendingNotification, PushDeviceState, PushEnvironment, PushTokenKind,
    UserId, VaultId,
};
use folio_infra::{Database, Repositories};
use testcontainers_modules::postgres::Postgres;
use testcontainers_modules::testcontainers::runners::AsyncRunner;
use testcontainers_modules::testcontainers::ImageExt;
use uuid::Uuid;

async fn user(auth: &dyn AuthStore, id: &str) {
    auth.create_user(User {
        id: id.into(),
        username: id.into(),
        password_hash: "!".into(),
        created_at: 0,
        email: None,
        display_name: None,
        is_admin: false,
        is_owner: false,
        auth_source: "local".into(),
        external_id: None,
    })
    .await
    .unwrap();
}

/// A signed-in app: client, access token and refresh token sharing `grant`
async fn sign_in(auth: &dyn AuthStore, user: &str, client: &str, grant: &str) {
    if auth.find_client(client).await.unwrap().is_none() {
        auth.insert_client(OAuthClient {
            client_id: client.into(),
            client_name: "Folio".into(),
            redirect_uris: vec!["folio://callback".into()],
            created_at: 0,
        })
        .await
        .unwrap();
    }
    let expires_at = folio_auth::server::now() + 3600;
    auth.insert_access_token(AccessToken {
        token_hash: format!("access-{grant}"),
        client_id: client.into(),
        user_id: user.into(),
        scopes: vec!["documents:read".into()],
        audience: "folio".into(),
        expires_at,
        grant_id: grant.into(),
    })
    .await
    .unwrap();
    auth.insert_refresh_token(RefreshToken {
        token_hash: format!("refresh-{grant}"),
        client_id: client.into(),
        user_id: user.into(),
        scopes: vec!["documents:read".into()],
        audience: "folio".into(),
        expires_at,
        grant_id: grant.into(),
    })
    .await
    .unwrap();
}

fn registration(user: &str, installation: &str, client: &str, grant: &str) -> DeviceRegistration {
    DeviceRegistration {
        user_id: UserId::from(user),
        installation_id: installation.into(),
        client_id: client.into(),
        grant_id: grant.into(),
        platform: DevicePlatform::Ios,
        environment: PushEnvironment::Production,
        locale: "en".into(),
        preferences: NotificationPreferences::default(),
        alert_token: Some(format!("{:0>64}", installation.len())),
        widget_token: None,
    }
}

async fn document(repos: &Repositories, vault: VaultId) -> Uuid {
    repos
        .documents
        .create(
            vault,
            CreateDocument {
                title: "Invoice".into(),
                ..Default::default()
            },
        )
        .await
        .unwrap()
        .id
}

fn item(
    device: Uuid,
    document: Uuid,
    vault: VaultId,
    at: chrono::DateTime<Utc>,
) -> PendingNotification {
    PendingNotification {
        device_id: device,
        document_id: document,
        kind: NotificationKind::NeedsReview,
        vault_id: vault,
        created_at: at,
    }
}

async fn contract(db: &Database) {
    db.migrate().await.unwrap();
    let repos = db.repositories().await;
    let (push, auth) = (repos.push.clone(), repos.auth.clone());
    user(&*auth, "alice").await;
    user(&*auth, "bob").await;
    sign_in(&*auth, "alice", "app", "g1").await;
    let vault = repos.vaults.create("Inbox", None).await.unwrap().id;

    // Registration and token refresh
    let (first, created) = push
        .upsert(registration("alice", "phone", "app", "g1"))
        .await
        .unwrap();
    assert!(created);
    assert_eq!(first.preferences, NotificationPreferences::default());
    let alert = first.alert_token.clone().unwrap();
    let mut refresh = registration("alice", "phone", "app", "g1");
    refresh.alert_token = None;
    refresh.widget_token = Some("cd".repeat(32));
    refresh.locale = "de".into();
    let (again, created) = push.upsert(refresh).await.unwrap();
    assert!(!created);
    assert_eq!(again.id, first.id);
    assert_eq!(again.locale, "de");
    assert_eq!(
        again.alert_token,
        Some(alert.clone()),
        "an absent token is kept with its time"
    );
    assert!(again.widget_token.is_some());

    // A token moving to another person deletes the row it left empty
    let mut bobs = registration("bob", "ipad", "app", "g2");
    bobs.alert_token = Some(alert.value.clone());
    let (bob, _) = push.upsert(bobs).await.unwrap();
    let alice = push.get(first.id).await.unwrap().unwrap();
    assert_eq!(alice.alert_token, None);
    assert_eq!(bob.alert_token.as_ref().unwrap().value, alert.value);
    let mut bobs = registration("bob", "ipad", "app", "g2");
    bobs.alert_token = Some("ef".repeat(32));
    push.upsert(bobs).await.unwrap();
    let mut alices = registration("alice", "phone", "app", "g1");
    alices.alert_token = Some(alert.value.clone());
    push.upsert(alices).await.unwrap();
    assert_eq!(
        push.list_for_user(&UserId::from("alice"))
            .await
            .unwrap()
            .len(),
        1
    );
    assert_eq!(push.count_devices().await.unwrap(), 2);

    // First outcomes and pending rows
    let doc = document(&repos, vault).await;
    let now = Utc::now();
    let rows = [
        item(first.id, doc, vault, now),
        item(first.id, doc, vault, now),
    ];
    assert!(push
        .record_first_outcome(doc, DocumentOutcome::Completed, &rows)
        .await
        .unwrap());
    assert!(!push
        .record_first_outcome(doc, DocumentOutcome::Completed, &rows)
        .await
        .unwrap());
    assert!(push
        .record_first_outcome(doc, DocumentOutcome::Failed, &[])
        .await
        .unwrap());
    assert_eq!(
        push.pending(first.id).await.unwrap().len(),
        1,
        "(device, document, kind) dedupes"
    );
    assert!(
        !push
            .record_first_outcome(Uuid::new_v4(), DocumentOutcome::Completed, &[])
            .await
            .unwrap(),
        "an unknown document records nothing"
    );

    // Claims: quiet window, ceiling and lease
    let quiet = Duration::seconds(30);
    let ceiling = Duration::minutes(5);
    let lease = Duration::seconds(60);
    assert!(push
        .claim_due(now + Duration::seconds(10), quiet, ceiling, lease, 10)
        .await
        .unwrap()
        .is_empty());
    let later = now + Duration::seconds(31);
    let (a, b) = tokio::join!(
        push.claim_due(later, quiet, ceiling, lease, 10),
        push.claim_due(later, quiet, ceiling, lease, 10)
    );
    let mut claimed = a.unwrap();
    claimed.extend(b.unwrap());
    assert_eq!(
        claimed,
        vec![first.id],
        "two claims never return the same device"
    );
    let state = PushDeviceState {
        alerts_in_window: 3,
        window_started_at: Some(later),
        backoff_until: Some(later + Duration::minutes(1)),
        failures: 1,
        ..Default::default()
    };
    push.release(first.id, &state).await.unwrap();
    let stored = push.state(first.id).await.unwrap();
    assert_eq!(stored.alerts_in_window, 3);
    assert_eq!(stored.failures, 1);
    assert!(
        push.claim_due(later, quiet, ceiling, lease, 10)
            .await
            .unwrap()
            .is_empty(),
        "backing off"
    );
    assert_eq!(
        push.claim_due(later + Duration::minutes(2), quiet, ceiling, lease, 10)
            .await
            .unwrap(),
        vec![first.id]
    );
    push.release(first.id, &PushDeviceState::default())
        .await
        .unwrap();
    push.remove_pending(first.id, &[(doc, NotificationKind::NeedsReview)])
        .await
        .unwrap();
    assert!(push.pending(first.id).await.unwrap().is_empty());

    // Widgets and test notifications
    push.mark_widgets_dirty(&[first.id], now).await.unwrap();
    assert_eq!(
        push.claim_widget_due(now, Duration::minutes(15), lease, 10)
            .await
            .unwrap(),
        vec![first.id]
    );
    push.release(
        first.id,
        &PushDeviceState {
            widgets_sent_at: Some(now),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert!(push
        .claim_widget_due(now, Duration::minutes(15), lease, 10)
        .await
        .unwrap()
        .is_empty());
    assert!(push
        .try_record_test(first.id, now, now - Duration::minutes(1))
        .await
        .unwrap());
    assert!(!push
        .try_record_test(first.id, now, now - Duration::minutes(1))
        .await
        .unwrap());

    // Token invalidation compares against each token's own registration time
    let device = push.get(first.id).await.unwrap().unwrap();
    let registered = device.alert_token.unwrap().registered_at;
    assert!(!push
        .clear_token(
            first.id,
            PushTokenKind::Alert,
            Some(registered - Duration::hours(1))
        )
        .await
        .unwrap());
    assert!(push
        .clear_token(
            first.id,
            PushTokenKind::Alert,
            Some(registered + Duration::hours(1))
        )
        .await
        .unwrap());
    assert!(push
        .get(first.id)
        .await
        .unwrap()
        .unwrap()
        .widget_token
        .is_some());
    assert!(push
        .clear_token(first.id, PushTokenKind::Widget, None)
        .await
        .unwrap());
    assert!(
        push.get(first.id).await.unwrap().is_none(),
        "no token left deletes the device"
    );

    // Pruning and document deletion
    let (phone, _) = push
        .upsert(registration("alice", "phone", "app", "g1"))
        .await
        .unwrap();
    let old = document(&repos, vault).await;
    let gone = document(&repos, vault).await;
    push.record_first_outcome(
        old,
        DocumentOutcome::Completed,
        &[item(phone.id, old, vault, now - Duration::days(2))],
    )
    .await
    .unwrap();
    push.record_first_outcome(
        gone,
        DocumentOutcome::Completed,
        &[item(phone.id, gone, vault, now)],
    )
    .await
    .unwrap();
    assert_eq!(
        push.prune_pending(now - Duration::days(1)).await.unwrap(),
        1
    );
    match db {
        Database::Sqlite(pool) => {
            sqlx::query("DELETE FROM documents WHERE id = ?")
                .bind(gone.to_string())
                .execute(pool)
                .await
                .unwrap();
        }
        Database::Postgres(pool) => {
            sqlx::query("DELETE FROM documents WHERE id = $1")
                .bind(gone)
                .execute(pool)
                .await
                .unwrap();
        }
    }
    assert!(
        push.pending(phone.id).await.unwrap().is_empty(),
        "deleting a document drops its rows"
    );

    // Grant liveness ignores used and expired tokens
    let grants = repos.grants.clone();
    assert!(grants.is_live("g1").await.unwrap());
    assert!(!grants.is_live("unknown").await.unwrap());
    assert!(!grants.is_live("").await.unwrap());
    match db {
        Database::Sqlite(pool) => {
            sqlx::query("DELETE FROM oauth_access_tokens WHERE grant_id = 'g1'")
                .execute(pool)
                .await
                .unwrap();
            sqlx::query("UPDATE oauth_refresh_tokens SET used = 1 WHERE grant_id = 'g1'")
                .execute(pool)
                .await
                .unwrap();
        }
        Database::Postgres(pool) => {
            sqlx::query("DELETE FROM oauth_access_tokens WHERE grant_id = 'g1'")
                .execute(pool)
                .await
                .unwrap();
            sqlx::query("UPDATE oauth_refresh_tokens SET used = 1 WHERE grant_id = 'g1'")
                .execute(pool)
                .await
                .unwrap();
        }
    }
    assert!(
        !grants.is_live("g1").await.unwrap(),
        "only a used refresh token is left"
    );

    // Revocation deletes devices in the same transaction
    sign_in(&*auth, "alice", "app", "g3").await;
    push.upsert(registration("alice", "phone", "app", "g3"))
        .await
        .unwrap();
    auth.revoke_token("access-g3", "app").await.unwrap();
    assert!(
        push.find(&UserId::from("alice"), "phone")
            .await
            .unwrap()
            .is_none(),
        "sign-out"
    );

    sign_in(&*auth, "alice", "app", "g4").await;
    push.upsert(registration("alice", "phone", "app", "g4"))
        .await
        .unwrap();
    sign_in(&*auth, "alice", "other-app", "g5").await;
    push.upsert(registration("alice", "mac", "other-app", "g5"))
        .await
        .unwrap();
    auth.revoke_client_tokens("alice", "app").await.unwrap();
    assert!(
        push.find(&UserId::from("alice"), "phone")
            .await
            .unwrap()
            .is_none(),
        "revoked on the web"
    );
    assert!(
        push.find(&UserId::from("alice"), "mac")
            .await
            .unwrap()
            .is_some(),
        "other clients stay"
    );

    auth.revoke_grant("g5").await.unwrap();
    assert!(
        push.find(&UserId::from("alice"), "mac")
            .await
            .unwrap()
            .is_none(),
        "refresh reuse"
    );

    // Deleting a user deletes their devices
    assert!(push
        .find(&UserId::from("bob"), "ipad")
        .await
        .unwrap()
        .is_some());
    match db {
        Database::Sqlite(pool) => {
            sqlx::query("DELETE FROM users WHERE id = 'bob'")
                .execute(pool)
                .await
                .unwrap();
        }
        Database::Postgres(pool) => {
            sqlx::query("DELETE FROM users WHERE id = 'bob'")
                .execute(pool)
                .await
                .unwrap();
        }
    }
    assert_eq!(push.count_devices().await.unwrap(), 0);

    // Deleting a device
    push.upsert(registration("alice", "phone", "app", "g4"))
        .await
        .unwrap();
    assert!(push.delete(&UserId::from("alice"), "phone").await.unwrap());
    assert!(!push.delete(&UserId::from("alice"), "phone").await.unwrap());
}

#[tokio::test]
async fn sqlite_push_devices() {
    let dir = tempfile::tempdir().unwrap();
    let url = format!("sqlite:{}?mode=rwc", dir.path().join("push.db").display());
    contract(&Database::connect(&url).await.unwrap()).await;
}

#[tokio::test]
async fn postgres_push_devices() {
    let started = Postgres::default().with_tag("17-alpine").start().await;
    let Some(container) = common::container_or_skip("PostgreSQL", started) else {
        return;
    };
    let host = container.get_host().await.unwrap();
    let port = container.get_host_port_ipv4(5432).await.unwrap();
    contract(
        &Database::connect(&format!(
            "postgres://postgres:postgres@{host}:{port}/postgres"
        ))
        .await
        .unwrap(),
    )
    .await;
}
