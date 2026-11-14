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
