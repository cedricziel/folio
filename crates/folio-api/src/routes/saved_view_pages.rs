use askama::Template;
use axum::{
    extract::{Path, RawForm, RawQuery, State},
    response::{Html, Redirect},
    routing::{get, post},
    Router,
};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use folio_core::models::{
    SavedView, SavedViewInput, SavedViewQuery, ViewPresentation, ViewVisibility,
};
use uuid::Uuid;

use super::{
    saved_views,
    view::{self, DocumentRow, Nav, Sidebar},
};
use crate::{
    error::{ApiError, ApiResult},
    state::AppState,
    vault::{filters, DefaultVault, Uploader},
};

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/saved-views", get(index).post(create))
        .route("/saved-views/new", get(new))
        .route("/saved-views/{id}", get(show).post(update))
        .route("/saved-views/{id}/delete", post(delete))
}

#[derive(Template)]
#[template(path = "saved_views.html")]
struct Index {
    sidebar: Sidebar,
    views: Vec<SavedView>,
    next: String,
}

async fn sidebar(state: &AppState, vault: folio_core::models::VaultId) -> ApiResult<Sidebar> {
    let mut sidebar = Sidebar::build(state, vault, Nav::None, &[]).await?;
    sidebar.show_counts = false;
    Ok(sidebar)
}

async fn index(
    DefaultVault(vault): DefaultVault,
    uploader: Uploader,
    State(state): State<AppState>,
    RawQuery(raw): RawQuery,
) -> ApiResult<Html<String>> {
    let access = saved_views::access(&state, vault, uploader).await?;
    let after = url::form_urlencoded::parse(raw.as_deref().unwrap_or_default().as_bytes())
        .find(|(key, _)| key == "after")
        .map(|(_, value)| value.parse::<Uuid>())
        .transpose()
        .map_err(|_| ApiError::bad_request("Invalid page"))?;
    let views = state.saved_views.list(vault, &access, after, 50).await?;
    let next = if views.len() == 50 {
        views
            .last()
            .map(|view| view.id.to_string())
            .unwrap_or_default()
    } else {
        String::new()
    };
    render(Index {
        sidebar: sidebar(&state, vault).await?,
        views,
        next,
    })
}

fn render(template: impl Template) -> ApiResult<Html<String>> {
    template
        .render()
        .map(Html)
        .map_err(|_| ApiError::internal("Could not render saved views"))
}

struct Choice {
    id: String,
    name: String,
    selected: bool,
}

#[derive(Template)]
#[template(path = "saved_view.html")]
struct Editor {
    query_error: Option<String>,
    sidebar: Sidebar,
    id: String,
    name: String,
    search: String,
    reviewed: String,
    document_date_from: String,
    document_date_to: String,
    sort: String,
    presentation: String,
    shared: bool,
    can_share: bool,
    can_edit: bool,
    after: String,
    before: String,
    after_original: String,
    before_original: String,
    correspondents: Vec<Choice>,
    types: Vec<Choice>,
    tags: Vec<Choice>,
    rows: Vec<DocumentRow>,
    next: String,
    is_saved: bool,
}

fn preserve_missing(choices: &mut Vec<Choice>, ids: impl IntoIterator<Item = Uuid>, label: &str) {
    for id in ids {
        let id = id.to_string();
        if !choices.iter().any(|choice| choice.id == id) {
            choices.push(Choice {
                id,
                name: format!("Unavailable {label}"),
                selected: true,
            });
        }
    }
}

async fn editor(
    state: &AppState,
    vault: folio_core::models::VaultId,
    access: &folio_core::ports::SavedViewAccess,
    view: Option<SavedView>,
    query: SavedViewQuery,
    cursor: Option<String>,
) -> ApiResult<Html<String>> {
    let query_error = match saved_views::validate_references(state, vault, &query).await {
        Ok(()) => None,
        Err(error) if view.is_some() => Some(error.message().to_owned()),
        Err(error) => return Err(error),
    };
    let cursor = cursor
        .map(|value| {
            URL_SAFE_NO_PAD
                .decode(value)
                .map_err(|_| ApiError::bad_request("Invalid cursor"))
        })
        .transpose()?
        .map(|bytes| {
            serde_json::from_slice(&bytes).map_err(|_| ApiError::bad_request("Invalid cursor"))
        })
        .transpose()?;
    let page = if query_error.is_some() {
        folio_core::models::DocumentPage {
            documents: Vec::new(),
            next_cursor: None,
        }
    } else {
        state
            .document_repo
            .page(vault, query.page(vault, 50, cursor)?)
            .await?
    };
    let next = page
        .next_cursor
        .map(|cursor| serde_json::to_vec(&cursor).map(|bytes| URL_SAFE_NO_PAD.encode(bytes)))
        .transpose()
        .map_err(|_| ApiError::internal("Could not encode cursor"))?
        .unwrap_or_default();
    let sidebar = sidebar(state, vault).await?;
    let mut correspondents: Vec<Choice> = sidebar
        .correspondents
        .iter()
        .map(|item| Choice {
            id: item.id.to_string(),
            name: item.name.clone(),
            selected: query.filters.correspondent_id == Some(item.id),
        })
        .collect();
    let mut types: Vec<Choice> = sidebar
        .document_types
        .iter()
        .map(|item| Choice {
            id: item.id.to_string(),
            name: item.name.clone(),
            selected: query.filters.document_type_id == Some(item.id),
        })
        .collect();
    let mut tags: Vec<Choice> = sidebar
        .tags
        .iter()
        .map(|item| Choice {
            id: item.id.to_string(),
            name: item.name.clone(),
            selected: query
                .filters
                .tag_ids
                .as_ref()
                .is_some_and(|ids| ids.contains(&item.id)),
        })
        .collect();
    preserve_missing(&mut correspondents, query.filters.correspondent_id, "From");
    preserve_missing(&mut types, query.filters.document_type_id, "type");
    preserve_missing(
        &mut tags,
        query.filters.tag_ids.iter().flatten().copied(),
        "tag",
    );
    let can_edit = view
        .as_ref()
        .is_none_or(|view| view.editable_by(&access.user, access.role));
    let sort = serde_json::to_value(query.sort)
        .map_err(|_| ApiError::internal("Invalid sort"))?
        .as_str()
        .unwrap_or("newest")
        .to_owned();
    render(Editor {
        query_error,
        sidebar,
        id: view
            .as_ref()
            .map(|view| view.id.to_string())
            .unwrap_or_default(),
        name: view
            .as_ref()
            .map(|view| view.name.clone())
            .unwrap_or_default(),
        search: query.filters.search_query.unwrap_or_default(),
        reviewed: query
            .filters
            .reviewed
            .map(|value| value.to_string())
            .unwrap_or_default(),
        document_date_from: query
            .filters
            .document_date_from
            .map(|date| date.to_string())
            .unwrap_or_default(),
        document_date_to: query
            .filters
            .document_date_to
            .map(|date| date.to_string())
            .unwrap_or_default(),
        sort,
        presentation: if view
            .as_ref()
            .is_some_and(|view| view.presentation == ViewPresentation::Grid)
        {
            "grid"
        } else {
            "list"
        }
        .into(),
        shared: view
            .as_ref()
            .is_some_and(|view| view.visibility == ViewVisibility::Shared),
        can_share: access.role >= folio_core::models::VaultRole::Editor,
        can_edit,
        after_original: query
            .filters
            .created_after
            .map(|date| date.to_rfc3339())
            .unwrap_or_default(),
        before_original: query
            .filters
            .created_before
            .map(|date| date.to_rfc3339())
            .unwrap_or_default(),
        after: query
            .filters
            .created_after
            .map(|date| date.format("%Y-%m-%dT%H:%M:%S%.3f").to_string())
            .unwrap_or_default(),
        before: query
            .filters
            .created_before
            .map(|date| date.format("%Y-%m-%dT%H:%M:%S%.3f").to_string())
            .unwrap_or_default(),
        correspondents,
        types,
        tags,
        rows: view::rows(DefaultVault(vault), state, page.documents).await?,
        next,
        is_saved: view.is_some(),
    })
}

async fn new(
    DefaultVault(vault): DefaultVault,
    uploader: Uploader,
    State(state): State<AppState>,
    RawQuery(raw): RawQuery,
) -> ApiResult<Html<String>> {
    let access = saved_views::access(&state, vault, uploader).await?;
    let query = super::browse::query(raw.as_deref())?;
    editor(
        &state,
        vault,
        &access,
        None,
        SavedViewQuery {
            version: 1,
            filters: query.filter,
            sort: query.sort,
        },
        None,
    )
    .await
}

async fn show(
    DefaultVault(vault): DefaultVault,
    uploader: Uploader,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    RawQuery(raw): RawQuery,
) -> ApiResult<Html<String>> {
    let access = saved_views::access(&state, vault, uploader).await?;
    let view = saved_views::visible(&state, vault, &access, id).await?;
    let cursor = url::form_urlencoded::parse(raw.as_deref().unwrap_or_default().as_bytes())
        .find(|(key, _)| key == "cursor")
        .map(|(_, value)| value.into_owned());
    editor(
        &state,
        vault,
        &access,
        Some(view.clone()),
        view.query,
        cursor,
    )
    .await
}

fn input(bytes: &[u8]) -> ApiResult<SavedViewInput> {
    let values: Vec<_> = url::form_urlencoded::parse(bytes)
        .map(|(key, value)| (key.into_owned(), value.into_owned()))
        .collect();
    let field = |name: &str| {
        values
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.as_str())
            .unwrap_or_default()
    };
    let visibility = match field("visibility") {
        "private" => ViewVisibility::Private,
        "shared" => ViewVisibility::Shared,
        _ => return Err(ApiError::bad_request("Invalid visibility")),
    };
    let presentation = match field("presentation") {
        "list" => ViewPresentation::List,
        "grid" => ViewPresentation::Grid,
        _ => return Err(ApiError::bad_request("Invalid presentation")),
    };
    let mut query = url::form_urlencoded::Serializer::new(String::new());
    for (key, value) in &values {
        if value.is_empty() || key == "created_after_original" || key == "created_before_original" {
            continue;
        }
        if key == "created_after" || key == "created_before" {
            let date = chrono::NaiveDateTime::parse_from_str(value, "%Y-%m-%dT%H:%M:%S%.f")
                .or_else(|_| chrono::NaiveDateTime::parse_from_str(value, "%Y-%m-%dT%H:%M"))
                .map_err(|_| ApiError::bad_request("Invalid date"))?;
            let date = date.and_utc();
            let original = chrono::DateTime::parse_from_rfc3339(field(&format!("{key}_original")))
                .ok()
                .map(|date| date.with_timezone(&chrono::Utc));
            let preserved = original.filter(|original| {
                chrono::DateTime::from_timestamp_millis(original.timestamp_millis()) == Some(date)
            });
            query.append_pair(key, &preserved.unwrap_or(date).to_rfc3339());
        } else {
            query.append_pair(key, value);
        }
    }
    let page = super::browse::query(Some(&query.finish()))?;
    Ok(SavedViewInput {
        name: field("name").into(),
        visibility,
        query: SavedViewQuery {
            version: 1,
            filters: page.filter,
            sort: page.sort,
        },
        presentation,
    })
}

async fn create(
    DefaultVault(vault): DefaultVault,
    uploader: Uploader,
    State(state): State<AppState>,
    RawForm(form): RawForm,
) -> ApiResult<Redirect> {
    let access = saved_views::access(&state, vault, uploader).await?;
    let input = input(&form)?;
    saved_views::validate(&state, vault, &input, &access).await?;
    let view = state.saved_views.create(vault, &access, input).await?;
    Ok(Redirect::to(&crate::vault::url(&format!(
        "/saved-views/{}",
        view.id
    ))))
}

async fn update(
    DefaultVault(vault): DefaultVault,
    uploader: Uploader,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    RawForm(form): RawForm,
) -> ApiResult<Redirect> {
    let access = saved_views::access(&state, vault, uploader).await?;
    let view = saved_views::visible(&state, vault, &access, id).await?;
    saved_views::may_edit(&view, &access)?;
    let input = input(&form)?;
    saved_views::validate(&state, vault, &input, &access).await?;
    state.saved_views.update(vault, &access, id, input).await?;
    Ok(Redirect::to(&crate::vault::url(&format!(
        "/saved-views/{id}"
    ))))
}

async fn delete(
    DefaultVault(vault): DefaultVault,
    uploader: Uploader,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> ApiResult<Redirect> {
    let access = saved_views::access(&state, vault, uploader).await?;
    let view = saved_views::visible(&state, vault, &access, id).await?;
    saved_views::may_edit(&view, &access)?;
    state.saved_views.delete(vault, &access, id).await?;
    Ok(Redirect::to(&crate::vault::url("/saved-views")))
}
