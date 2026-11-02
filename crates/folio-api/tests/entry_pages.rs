mod common;

use axum::{
    body::Body,
    http::{header, Request, StatusCode},
    response::Response,
};
use folio_auth::{crypto::hash_token, Session, User};
use folio_core::models::{CreateDocument, Principal, UserId, VaultId, VaultRole};

use tower::ServiceExt;
use uuid::Uuid;

async fn member(h: &common::Harness, name: &str, vault: VaultId, role: VaultRole) {
    h.auth_store
        .create_user(User {
            id: name.into(),
            username: name.into(),
            password_hash: "!".into(),
            created_at: 1,
            email: None,
            display_name: None,
            is_admin: false,
            is_owner: false,
            auth_source: "local".into(),
            external_id: None,
        })
        .await
        .unwrap();
    h.auth_store
        .create_session(Session {
            id_hash: hash_token(name),
            user_id: name.into(),
            expires_at: i64::MAX / 2,
            created_at: 1,
            user_agent: None,
        })
        .await
        .unwrap();
    h.members
        .grant(vault, &Principal::User(UserId(name.into())), role, None)
        .await
        .unwrap();
}

async fn request(
    h: &common::Harness,
    actor: &str,
    method: &str,
    path: &str,
    body: &str,
) -> Response {
    h.app
        .clone()
        .oneshot(
            Request::builder()
                .method(method)
                .uri(path)
                .header(header::HOST, "localhost")
                .header(header::ORIGIN, "http://localhost")
                .header(header::COOKIE, format!("folio_session={actor}"))
                .header("x-folio-vault", h.vault.to_string())
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .body(Body::from(body.to_owned()))
                .unwrap(),
        )
        .await
        .unwrap()
}

#[tokio::test]
async fn html_entries_escape_text_preserve_failed_drafts_and_enforce_permissions() {
    let h = common::harness_requiring_login().await;
    member(&h, "author", h.vault, VaultRole::Contributor).await;
    member(&h, "viewer", h.vault, VaultRole::Viewer).await;
    member(&h, "editor", h.vault, VaultRole::Editor).await;
    let document = h
        .state
        .document_repo
        .create(
            h.vault,
            CreateDocument {
                title: "Discussion".into(),
                uploaded_by: Some(UserId("editor".into())),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let path = format!("/documents/{}/entries", document.id);
    let page = request(&h, "author", "GET", &format!("{path}?kind=note"), "").await;
    assert_eq!(page.status(), StatusCode::OK);
    let html = common::text(page).await;
    assert!(html.contains("No notes yet"));
    assert!(html.contains("Add note"));
    let key = Uuid::new_v4();
    let form =
        format!("kind=note&body=%3Cscript%3Ealert%281%29%3C%2Fscript%3E&idempotency_key={key}");
    assert_eq!(
        request(&h, "author", "POST", &path, &form).await.status(),
        StatusCode::SEE_OTHER
    );
    assert_eq!(
        request(&h, "author", "POST", &path, &form).await.status(),
        StatusCode::SEE_OTHER
    );
    let page =
        common::text(request(&h, "viewer", "GET", &format!("{path}?kind=note"), "").await).await;
    assert!(
        page.contains("&lt;script&gt;") || page.contains("&#60;script&#62;"),
        "{page}"
    );
    assert!(!page.contains("<script>alert"));
    assert!(!page.contains("Add note"));
    assert!(!page.contains("Delete note"));
    let conflict = format!("kind=note&body=Keep+this+draft&idempotency_key={key}");
    let response = request(&h, "author", "POST", &path, &conflict).await;
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let html = common::text(response).await;
    assert!(html.contains("Keep this draft"));
    assert!(html.contains(&key.to_string()));
    assert!(html.contains("role=\"alert\""));
    let entries = h
        .state
        .entry_repo
        .page(
            h.vault,
            document.id,
            folio_core::models::EntryPageQuery {
                kind: folio_core::models::EntryKind::Note,
                limit: 20,
                cursor: None,
            },
        )
        .await
        .unwrap();
    assert_eq!(entries.entries.len(), 1);
    let id = entries.entries[0].id;
    let failed_edit = request(&h, "author", "POST", &format!("{path}/{id}/edit"), "body=+").await;
    assert_eq!(failed_edit.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let failed_html = common::text(failed_edit).await;
    assert_eq!(
        failed_html
            .matches(&format!("action=\"/vaults/{}{path}/{id}/edit\"", h.vault))
            .count(),
        1
    );
    let retry_form = failed_html.split("Retry save").next().unwrap();
    assert!(retry_form.contains("name=\"csrf\""));

    assert_eq!(
        request(
            &h,
            "editor",
            "POST",
            &format!("{path}/{id}/edit"),
            "body=Changed"
        )
        .await
        .status(),
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        request(
            &h,
            "author",
            "POST",
            &format!("{path}/{id}/edit"),
            "body=Changed"
        )
        .await
        .status(),
        StatusCode::SEE_OTHER
    );
    assert_eq!(
        request(&h, "editor", "POST", &format!("{path}/{id}/delete"), "")
            .await
            .status(),
        StatusCode::SEE_OTHER
    );
    assert_eq!(
        request(
            &h,
            "editor",
            "POST",
            &format!("{path}/{id}/delete"),
            "kind=note"
        )
        .await
        .status(),
        StatusCode::SEE_OTHER
    );
    assert!(h
        .state
        .entry_repo
        .find(h.vault, document.id, id)
        .await
        .unwrap()
        .is_none());
}

#[tokio::test]
async fn html_pages_are_bounded_and_cursors_remain_scoped() {
    use folio_core::{
        models::{EntryBody, EntryKind},
        mutation::with_actor,
    };
    let h = common::harness_requiring_login().await;
    member(&h, "author", h.vault, VaultRole::Contributor).await;
    let document = h
        .state
        .document_repo
        .create(
            h.vault,
            CreateDocument {
                title: "Paged notes".into(),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    for index in 0..21 {
        with_actor(
            Some(UserId("author".into())),
            h.state.entry_repo.create(
                h.vault,
                document.id,
                EntryKind::Note,
                EntryBody::parse(format!("Note {index}")).unwrap(),
                Uuid::new_v4(),
            ),
        )
        .await
        .unwrap();
    }
    let path = format!("/documents/{}/entries", document.id);
    let html =
        common::text(request(&h, "author", "GET", &format!("{path}?kind=note"), "").await).await;
    assert_eq!(html.matches("<article class=\"entry\"").count(), 20);
    let cursor = html
        .split("&amp;cursor=")
        .nth(1)
        .unwrap()
        .split('"')
        .next()
        .unwrap();
    let next = request(
        &h,
        "author",
        "GET",
        &format!("{path}?kind=note&cursor={cursor}"),
        "",
    )
    .await;
    assert_eq!(next.status(), StatusCode::OK);
    let next = common::text(next).await;
    assert_eq!(next.matches("<article class=\"entry\"").count(), 1);
    assert!(next.contains("First notes"));
    assert_eq!(
        request(
            &h,
            "author",
            "GET",
            &format!("{path}?kind=comment&cursor={cursor}"),
            ""
        )
        .await
        .status(),
        StatusCode::BAD_REQUEST
    );
}
