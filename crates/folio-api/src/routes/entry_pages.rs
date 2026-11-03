use crate::{
    error::{ApiError, ApiResult},
    state::AppState,
    vault::{filters, DefaultVault, CURRENT_VAULT},
};
use askama::Template;
use axum::{
    extract::{Form, Path, Query, State},
    http::StatusCode,
    response::{Html, IntoResponse, Redirect, Response},
    routing::{get, post},
    Router,
};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use folio_core::{
    models::{
        DocumentEntry, EntryBody, EntryKind, EntryPage, EntryPageQuery, EntryPermissions, VaultRole,
    },
    ports::EntryCreation,
};
use serde::Deserialize;
use uuid::Uuid;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/documents/{id}/entries", get(list).post(create))
        .route("/documents/{id}/entries/{entry}/edit", post(update))
        .route("/documents/{id}/entries/{entry}/delete", post(delete))
}

#[derive(Deserialize)]
pub struct PageQuery {
    kind: EntryKind,
    cursor: Option<String>,
    focus: Option<Uuid>,
}
#[derive(Deserialize)]
pub struct EntryForm {
    kind: EntryKind,
    body: String,
    idempotency_key: Uuid,
}
#[derive(Deserialize)]
pub struct EditForm {
    body: String,
}
#[derive(Deserialize)]
pub struct DeleteForm {
    kind: Option<EntryKind>,
}

struct Row {
    entry: DocumentEntry,
    can_edit: bool,
    can_delete: bool,
    is_editing: bool,
}
#[derive(Template)]
#[template(path = "partials/entries.html")]
struct EntriesTemplate {
    document: Uuid,
    kind: &'static str,
    heading: &'static str,
    rows: Vec<Row>,
    next: Option<String>,
    has_cursor: bool,
    focused: bool,
    can_create: bool,
    draft: String,
    key: Uuid,
    error: Option<String>,
    edit_id: Option<Uuid>,
    csrf: String,
}
fn context() -> ApiResult<crate::vault::VaultContext> {
    CURRENT_VAULT
        .try_with(Clone::clone)
        .map_err(|_| ApiError::new(StatusCode::UNAUTHORIZED, "Vault context required"))
}
fn permissions(entry: &DocumentEntry) -> ApiResult<EntryPermissions> {
    let ctx = context()?;
    Ok(ctx.user.as_ref().map_or(
        EntryPermissions {
            can_edit: false,
            can_delete: ctx.role >= VaultRole::Editor,
        },
        |user| EntryPermissions::for_member(ctx.role, user, entry.created_by.as_ref()),
    ))
}
struct Draft {
    body: String,
    key: Uuid,
    error: Option<String>,
    edit_id: Option<Uuid>,
}

async fn render(
    state: &AppState,
    vault: folio_core::models::VaultId,
    document: Uuid,
    query: PageQuery,
    draft: Draft,
) -> ApiResult<Html<String>> {
    if query.focus.is_some() && query.cursor.is_some() {
        return Err(ApiError::bad_request(
            "Choose either an entry or a page cursor",
        ));
    }
    let has_cursor = query.cursor.is_some() || query.focus.is_some();
    let cursor = query
        .cursor
        .map(|value| {
            URL_SAFE_NO_PAD
                .decode(value)
                .map_err(|_| ApiError::bad_request("Invalid cursor"))
                .and_then(|bytes| {
                    serde_json::from_slice(&bytes)
                        .map_err(|_| ApiError::bad_request("Invalid cursor"))
                })
        })
        .transpose()?;
    let page = if let Some(id) = query.focus {
        let entry = state
            .entry_repo
            .find(vault, document, id)
            .await?
            .filter(|entry| entry.kind == query.kind)
            .ok_or_else(|| ApiError::not_found("Entry not found"))?;
        EntryPage {
            entries: vec![entry],
            next_cursor: None,
        }
    } else {
        state
            .entry_repo
            .page(
                vault,
                document,
                EntryPageQuery {
                    kind: query.kind,
                    limit: 20,
                    cursor,
                },
            )
            .await?
    };
    let rows = page
        .entries
        .into_iter()
        .map(|entry| {
            let permission = permissions(&entry)?;
            let is_editing = draft.edit_id == Some(entry.id);
            Ok(Row {
                entry,
                is_editing,
                can_edit: permission.can_edit,
                can_delete: permission.can_delete,
            })
        })
        .collect::<ApiResult<Vec<_>>>()?;
    let next = page
        .next_cursor
        .map(|cursor| serde_json::to_vec(&cursor).map(|bytes| URL_SAFE_NO_PAD.encode(bytes)))
        .transpose()
        .map_err(|_| ApiError::internal("Could not encode cursor"))?;
    let template = EntriesTemplate {
        document,
        kind: query.kind.as_str(),
        heading: if query.kind == EntryKind::Note {
            "Notes"
        } else {
            "Comments"
        },
        rows,
        next,
        has_cursor,
        focused: query.focus.is_some(),
        can_create: context()?.role >= VaultRole::Contributor,
        draft: draft.body,
        key: draft.key,
        error: draft.error,
        edit_id: draft.edit_id,
        csrf: super::view::CURRENT_USER
            .try_with(|user| user.csrf.clone())
            .unwrap_or_default(),
    };
    Ok(Html(template.render().map_err(|_| {
        ApiError::internal("Could not render entries")
    })?))
}
async fn list(
    DefaultVault(vault): DefaultVault,
    State(state): State<AppState>,
    Path(document): Path<Uuid>,
    Query(query): Query<PageQuery>,
) -> ApiResult<Html<String>> {
    render(
        &state,
        vault,
        document,
        query,
        Draft {
            body: String::new(),
            key: Uuid::new_v4(),
            error: None,
            edit_id: None,
        },
    )
    .await
}
fn location(document: Uuid, kind: EntryKind) -> String {
    crate::vault::url(&format!(
        "/documents/{document}/entries?kind={}",
        kind.as_str()
    ))
}
fn focused_location(entry: &DocumentEntry) -> String {
    format!(
        "{}&focus={}",
        location(entry.document_id, entry.kind),
        entry.id
    )
}
async fn create(
    DefaultVault(vault): DefaultVault,
    State(state): State<AppState>,
    Path(document): Path<Uuid>,
    Form(form): Form<EntryForm>,
) -> ApiResult<Response> {
    if context()?.role < VaultRole::Contributor {
        return Err(ApiError::new(
            StatusCode::FORBIDDEN,
            "This vault is read-only",
        ));
    }
    let result = async {
        if form.idempotency_key.is_nil() {
            return Err(ApiError::bad_request("A draft key is required"));
        }
        let body = EntryBody::parse(form.body.clone())?;
        match state
            .entry_repo
            .create(vault, document, form.kind, body, form.idempotency_key)
            .await?
        {
            EntryCreation::Created(entry) | EntryCreation::Replayed(entry) => Ok(entry),
            EntryCreation::Deleted => Err(ApiError::new(
                StatusCode::GONE,
                "The original entry was deleted. Cancel this draft before starting another.",
            )),
        }
    }
    .await;
    match result {
        Ok(entry) => Ok(Redirect::to(&focused_location(&entry)).into_response()),
        Err(error) => Ok((
            StatusCode::UNPROCESSABLE_ENTITY,
            render(
                &state,
                vault,
                document,
                PageQuery {
                    kind: form.kind,
                    cursor: None,
                    focus: None,
                },
                Draft {
                    body: form.body,
                    key: form.idempotency_key,
                    error: Some(error.message().to_owned()),
                    edit_id: None,
                },
            )
            .await?,
        )
            .into_response()),
    }
}
async fn update(
    DefaultVault(vault): DefaultVault,
    State(state): State<AppState>,
    Path((document, id)): Path<(Uuid, Uuid)>,
    Form(form): Form<EditForm>,
) -> ApiResult<Response> {
    let current = state
        .entry_repo
        .find(vault, document, id)
        .await?
        .ok_or_else(|| ApiError::not_found("Entry not found"))?;
    if !permissions(&current)?.can_edit {
        return Err(ApiError::new(
            StatusCode::FORBIDDEN,
            "Only the author may edit this entry",
        ));
    }
    let result = async {
        Ok::<_, ApiError>(
            state
                .entry_repo
                .update(vault, document, id, EntryBody::parse(form.body.clone())?)
                .await?,
        )
    }
    .await;
    match result {
        Ok(entry) => Ok(Redirect::to(&focused_location(&entry)).into_response()),
        Err(error) => Ok((
            StatusCode::UNPROCESSABLE_ENTITY,
            render(
                &state,
                vault,
                document,
                PageQuery {
                    kind: current.kind,
                    cursor: None,
                    focus: None,
                },
                Draft {
                    body: form.body,
                    key: Uuid::new_v4(),
                    error: Some(error.message().to_owned()),
                    edit_id: Some(id),
                },
            )
            .await?,
        )
            .into_response()),
    }
}
async fn delete(
    DefaultVault(vault): DefaultVault,
    State(state): State<AppState>,
    Path((document, id)): Path<(Uuid, Uuid)>,
    Form(form): Form<DeleteForm>,
) -> ApiResult<Redirect> {
    let current = state.entry_repo.find(vault, document, id).await?;
    let kind = if let Some(current) = current {
        if !permissions(&current)?.can_delete {
            return Err(ApiError::new(
                StatusCode::FORBIDDEN,
                "You cannot delete this entry",
            ));
        }
        current.kind
    } else {
        form.kind.unwrap_or(EntryKind::Note)
    };
    if context()?.role < VaultRole::Contributor {
        return Err(ApiError::new(
            StatusCode::FORBIDDEN,
            "This vault is read-only",
        ));
    }
    state
        .entry_repo
        .delete(vault, document, id, context()?.role >= VaultRole::Editor)
        .await?;
    Ok(Redirect::to(&location(document, kind)))
}
