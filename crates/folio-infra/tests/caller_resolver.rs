//! Resolving callers from folio-auth accounts

use std::sync::Arc;

use folio_core::models::{Caller, UserId};
use folio_core::ports::CallerResolver;
use folio_infra::{AuthCallerResolver, Database};

async fn store_with_users() -> Arc<dyn folio_auth::AuthStore> {
    let db = Database::connect("sqlite::memory:").await.unwrap();
    db.migrate().await.unwrap();
    let store = db.repositories().await.auth;
    let user = |id: &str, is_admin: bool| folio_auth::User {
        id: id.into(),
        username: id.into(),
        password_hash: "!".into(),
        created_at: 1,
        email: None,
        display_name: None,
        is_admin,
        is_owner: false,
        auth_source: "local".into(),
        external_id: None,
    };
    store
        .create_first_user(user("owner", true))
        .await
        .unwrap()
        .unwrap();
    store.create_user(user("admin", true)).await.unwrap();
    store.create_user(user("member", false)).await.unwrap();
    store
}

#[tokio::test]
async fn accounts_resolve_with_their_instance_flags() {
    let resolver = AuthCallerResolver::new(store_with_users().await, true);
    let resolve = |id: &'static str| {
        let resolver = &resolver;
        async move { resolver.resolve(Some(&UserId::from(id))).await.unwrap() }
    };

    let owner = resolve("owner").await;
    assert!(owner.is_owner && owner.is_instance_admin());
    let admin = resolve("admin").await;
    assert!(admin.is_admin && !admin.is_owner);
    assert_eq!(admin.user_id, Some(UserId::from("admin")));
    let member = resolve("member").await;
    assert_eq!(member, Caller::unprivileged(Some(UserId::from("member"))));
}

#[tokio::test]
async fn unknown_and_anonymous_callers_have_no_rights_when_auth_is_required() {
    let resolver = AuthCallerResolver::new(store_with_users().await, true);
    let ghost = resolver
        .resolve(Some(&UserId::from("ghost")))
        .await
        .unwrap();
    assert!(!ghost.is_instance_admin());
    assert!(!resolver.resolve(None).await.unwrap().is_instance_admin());
}

#[tokio::test]
async fn everyone_is_an_admin_when_auth_is_disabled() {
    let resolver = AuthCallerResolver::new(store_with_users().await, false);
    let anonymous = resolver.resolve(None).await.unwrap();
    assert!(anonymous.is_admin && anonymous.user_id.is_none());
    let member = resolver
        .resolve(Some(&UserId::from("member")))
        .await
        .unwrap();
    assert!(member.is_instance_admin());
    assert_eq!(member.user_id, Some(UserId::from("member")));
}
