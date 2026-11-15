mod common;

use folio_core::models::CreateDocument;
use tower::ServiceExt;
use uuid::Uuid;

async fn page(h: &common::Harness, path: &str) -> String {
    let response = h.app.clone().oneshot(common::get(path)).await.unwrap();
    assert_eq!(response.status(), 200, "{path}");
    common::text(response).await
}

async fn document(h: &common::Harness) -> Uuid {
    h.state
        .document_repo
        .create(
            h.vault,
            CreateDocument {
                title: "Back link subject".into(),
                ..Default::default()
            },
        )
        .await
        .unwrap()
        .id
}

fn back_link(html: &str) -> (String, String) {
    let start = html.find("class=\"back-link\"").expect("back link");
    let tag_start = html[..start].rfind("<a ").unwrap();
    let end = start + html[start..].find("</a>").unwrap();
    let tag = &html[tag_start..end];
    let href = tag
        .split("href=\"")
        .nth(1)
        .unwrap()
        .split('"')
        .next()
        .unwrap();
    let label = tag.rsplit('>').next().unwrap().trim();
    (
        href.replace("&amp;", "&").replace("&#38;", "&"),
        label.to_owned(),
    )
}

#[tokio::test]
async fn list_rows_carry_their_list_as_the_return_target() {
    let h = common::harness().await;
    let id = document(&h).await;
    let list = format!("/vaults/{}/?view=all&reviewed=false", h.vault);
    let html = page(&h, &list).await;
    let href = html
        .split("href=\"")
        .map(|part| part.split('"').next().unwrap())
        .find(|href| href.contains(&format!("/documents/{id}")) && !href.contains("thumbnail"))
        .expect("row link");
    assert!(href.contains("return_to="), "{href}");
}

#[tokio::test]
async fn back_link_names_and_restores_the_origin_list() {
    let h = common::harness().await;
    let id = document(&h).await;
    let v = h.vault;
    let view = Uuid::new_v4();
    let cases = [
        (format!("/vaults/{v}/?view=all"), "Back to All documents"),
        (
            format!("/vaults/{v}/?view=all&tag_id={}", Uuid::new_v4()),
            "Back to filtered documents",
        ),
        (format!("/vaults/{v}/"), "Back to Inbox"),
        (format!("/vaults/{v}/trash"), "Back to Trash"),
        (format!("/vaults/{v}/saved-views"), "Back to saved views"),
        (
            format!("/vaults/{v}/saved-views/{view}?cursor=abc"),
            "Back to saved view",
        ),
    ];
    for (target, label) in cases {
        let encoded: String = url::form_urlencoded::byte_serialize(target.as_bytes()).collect();
        let html = page(
            &h,
            &format!("/vaults/{v}/documents/{id}?return_to={encoded}"),
        )
        .await;
        assert_eq!(
            back_link(&html),
            (target.clone(), label.to_owned()),
            "{target}"
        );
    }
}

#[tokio::test]
async fn back_link_falls_back_to_the_inbox() {
    let h = common::harness().await;
    let id = document(&h).await;
    let html = page(&h, &format!("/vaults/{}/documents/{id}", h.vault)).await;
    assert_eq!(
        back_link(&html),
        (format!("/vaults/{}/", h.vault), "Back to Inbox".to_owned())
    );
}

#[tokio::test]
async fn back_link_refuses_targets_outside_the_vault() {
    let h = common::harness().await;
    let id = document(&h).await;
    let other = Uuid::new_v4();
    for bad in [
        "https://evil.example/".to_owned(),
        "//evil.example/".to_owned(),
        "/\\evil.example".to_owned(),
        "javascript:alert(1)".to_owned(),
        "/api/tokens".to_owned(),
        format!("/vaults/{other}/?view=all"),
        format!("/vaults/{}/../settings", h.vault),
    ] {
        let encoded: String = url::form_urlencoded::byte_serialize(bad.as_bytes()).collect();
        let html = page(
            &h,
            &format!("/vaults/{}/documents/{id}?return_to={encoded}", h.vault),
        )
        .await;
        assert_eq!(
            back_link(&html),
            (format!("/vaults/{}/", h.vault), "Back to Inbox".to_owned()),
            "{bad}"
        );
    }
}

fn encode(value: &str) -> String {
    url::form_urlencoded::byte_serialize(value.as_bytes()).collect()
}

async fn post(h: &common::Harness, path: &str, body: String) -> axum::response::Response {
    h.app
        .clone()
        .oneshot(
            axum::http::Request::builder()
                .method("POST")
                .uri(path)
                .header("content-type", "application/x-www-form-urlencoded")
                .body(axum::body::Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap()
}

fn location(response: &axum::response::Response) -> String {
    response.headers()["location"].to_str().unwrap().to_owned()
}

#[tokio::test]
async fn empty_filter_values_are_not_filters() {
    let h = common::harness().await;
    let id = document(&h).await;
    let v = h.vault;
    for (target, label) in [
        ("?reviewed=&tag_id=", "Back to Inbox"),
        ("?reviewed=&tag_id=&view=all", "Back to All documents"),
        ("?reviewed=false", "Back to filtered documents"),
    ] {
        let target = format!("/vaults/{v}/{target}");
        let html = page(
            &h,
            &format!("/vaults/{v}/documents/{id}?return_to={}", encode(&target)),
        )
        .await;
        assert_eq!(back_link(&html).1, label, "{target}");
    }
}

#[tokio::test]
async fn hostile_targets_with_control_characters_encoding_or_length_are_refused() {
    let h = common::harness().await;
    let id = document(&h).await;
    let v = h.vault;
    for bad in [
        format!("/vaults/{v}/?view=all\r\nSet-Cookie: x=1"),
        format!("/vaults/{v}/\u{7}"),
        "/%2fevil.example".to_owned(),
        format!("/vaults/{v}/?view=all&x={}", "a".repeat(3000)),
    ] {
        let html = page(
            &h,
            &format!("/vaults/{v}/documents/{id}?return_to={}", encode(&bad)),
        )
        .await;
        assert_eq!(
            back_link(&html),
            (format!("/vaults/{v}/"), "Back to Inbox".to_owned()),
            "{bad}"
        );
    }
}

#[tokio::test]
async fn emitted_targets_leave_out_the_vault_prefix() {
    let h = common::harness().await;
    let id = document(&h).await;
    let html = page(&h, &format!("/vaults/{}/?view=all", h.vault)).await;
    let href = html
        .split("href=\"")
        .map(|part| part.split('"').next().unwrap())
        .find(|href| href.contains(&format!("/documents/{id}?")))
        .expect("row link");
    assert!(href.ends_with("return_to=%2F%3Fview%3Dall"), "{href}");
}

#[tokio::test]
async fn forms_carry_the_target_and_redirects_keep_it() {
    let h = common::harness().await;
    let id = document(&h).await;
    let v = h.vault;
    let list = format!("/vaults/{v}/?view=all&tag_id={}", Uuid::new_v4());
    let doc = format!("/vaults/{v}/documents/{id}");
    let html = page(&h, &format!("{doc}?return_to={}", encode(&list))).await;
    assert!(
        html.matches(r#"name="return_to""#).count() >= 4,
        "forms carry the target"
    );

    let kept = format!(
        "{doc}?return_to={}",
        encode(&list[format!("/vaults/{v}").len()..])
    );
    for (action, body) in [
        ("correspondent", "correspondent_id="),
        ("document_type", "document_type_id="),
        ("date", "clear=true"),
        ("review", "reviewed=true"),
    ] {
        let response = post(
            &h,
            &format!("{doc}/{action}"),
            format!("{body}&return_to={}", encode(&list)),
        )
        .await;
        assert!(response.status().is_redirection(), "{action}");
        assert_eq!(location(&response), kept, "{action}");
    }

    let response = post(
        &h,
        &format!("{doc}/correspondent"),
        "correspondent_id=&return_to=https%3A%2F%2Fevil.example".into(),
    )
    .await;
    assert_eq!(location(&response), doc);
}

#[tokio::test]
async fn deleting_returns_to_the_origin_list() {
    let h = common::harness().await;
    let v = h.vault;
    let list = format!("/vaults/{v}/?view=all");
    let id = document(&h).await;
    let response = post(
        &h,
        &format!("/vaults/{v}/documents/{id}/delete"),
        format!("return_to={}", encode(&list)),
    )
    .await;
    assert_eq!(location(&response), list);

    let id = document(&h).await;
    let response = post(
        &h,
        &format!("/vaults/{v}/documents/{id}/delete"),
        String::new(),
    )
    .await;
    assert_eq!(location(&response), format!("/vaults/{v}/"));
}
