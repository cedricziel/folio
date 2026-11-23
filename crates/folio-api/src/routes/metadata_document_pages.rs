use std::collections::BTreeMap;

use askama::Template;
use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::{Html, IntoResponse, Redirect, Response},
    routing::get,
    Form, Router,
};
use folio_core::models::{
    Document, DocumentMetadataValue, DocumentPageQuery, MetadataChange, MetadataField,
    MetadataKind, MetadataValue, VaultId,
};
use folio_core::Error;
use serde::Deserialize;
use uuid::Uuid;

use super::view::{Nav, Sidebar};
use crate::{
    error::{ApiError, ApiResult},
    state::AppState,
    vault::{filters, DefaultVault, Uploader},
};

const PAGE_SIZE: usize = 50;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/documents/{id}/custom-fields", get(panel))
        .route(
            "/documents/{id}/custom-fields/{field}",
            get(edit).post(save),
        )
}

#[derive(Default, Deserialize)]
struct FieldsQuery {
    after: Option<Uuid>,
    return_to: Option<String>,
}

struct FieldRow {
    name: String,
    display: String,
    href: Option<String>,
    edit_href: Option<String>,
    saved_view_href: Option<String>,
    retired: bool,
}

#[derive(Template)]
#[template(path = "partials/metadata_document_fields.html")]
struct FieldsPanel {
    rows: Vec<FieldRow>,
    next: Option<String>,
}

fn document_url(document: Uuid, return_to: Option<&str>) -> String {
    let href = crate::vault::url(&format!("/documents/{document}"));
    with_return(href, return_to)
}

fn with_return(href: String, return_to: Option<&str>) -> String {
    if let Some(back) = crate::back_link::resolve(return_to) {
        if back.target != "/" {
            let target =
                url::form_urlencoded::byte_serialize(back.target.as_bytes()).collect::<String>();
            let separator = if href.contains('?') { '&' } else { '?' };
            return format!("{href}{separator}return_to={target}");
        }
    }
    href
}

async fn document(state: &AppState, vault: VaultId, id: Uuid) -> ApiResult<Document> {
    state
        .document_repo
        .find_by_id(vault, id)
        .await?
        .ok_or_else(|| ApiError::not_found("Document not found"))
}

async fn display_value(
    state: &AppState,
    vault: VaultId,
    field: &MetadataField,
    assignment: Option<&DocumentMetadataValue>,
) -> ApiResult<(String, Option<String>)> {
    let Some((value, assignment)) = assignment
        .and_then(|assignment| assignment.value.as_ref().map(|value| (value, assignment)))
    else {
        let empty = if field.required && field.deleted_at.is_none() {
            "Required, not set"
        } else {
            "Not set"
        };
        return Ok((empty.into(), None));
    };
    let display = match value {
        MetadataValue::Text(text) | MetadataValue::LongText(text) if text.is_empty() => {
            "Empty text".into()
        }
        MetadataValue::Text(text)
        | MetadataValue::LongText(text)
        | MetadataValue::Integer(text)
        | MetadataValue::Decimal(text)
        | MetadataValue::Date(text) => text.clone(),
        MetadataValue::Boolean(value) => if *value { "Yes" } else { "No" }.into(),
        MetadataValue::Money { amount, currency } => format!("{amount} {currency}"),
        MetadataValue::Select(id) => match state.metadata_options.find(vault, field.id, *id).await?
        {
            Some(option) if option.deleted_at.is_some() => format!("{} (retired)", option.label),
            Some(option) => option.label,
            None => "Unavailable option".into(),
        },
        MetadataValue::Url(value) => {
            let href = url::Url::parse(value)
                .ok()
                .filter(|url| matches!(url.scheme(), "http" | "https"));
            return Ok((value.clone(), href.map(Into::into)));
        }
        MetadataValue::DocumentLink(id) => {
            return Ok(match &assignment.link_target {
                Some(target) if target.trashed => (format!("{} (in Trash)", target.title), None),
                Some(target) => (target.title.clone(), Some(document_url(*id, None))),
                None => ("Unavailable document".into(), None),
            })
        }
    };
    Ok((display, None))
}

async fn panel(
    DefaultVault(vault): DefaultVault,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Query(query): Query<FieldsQuery>,
    Uploader(user): Uploader,
) -> ApiResult<Html<String>> {
    let document = document(&state, vault, id).await?;
    let can_edit = crate::vault_context::can_edit_document(&document);
    let mut fields = state
        .metadata_fields
        .list(vault, true, query.after, PAGE_SIZE as u32 + 1)
        .await?;
    let has_more = fields.len() > PAGE_SIZE;
    fields.truncate(PAGE_SIZE);
    let next = fields.last().filter(|_| has_more).map(|last| {
        with_return(
            crate::vault::url(&format!("/documents/{id}/custom-fields?after={}", last.id)),
            query.return_to.as_deref(),
        )
    });
    let values: BTreeMap<_, _> = if fields.is_empty() {
        BTreeMap::new()
    } else {
        state
            .metadata_values
            .get(vault, id)
            .await?
            .values
            .into_iter()
            .map(|value| (value.field_id, value))
            .collect()
    };
    let mut rows = Vec::new();
    for field in fields {
        let assignment = values.get(&field.id);
        if field.deleted_at.is_some() && assignment.and_then(|value| value.value.as_ref()).is_none()
        {
            continue;
        }
        let (display, href) = display_value(&state, vault, &field, assignment).await?;
        let edit_href = can_edit.then(|| {
            with_return(
                crate::vault::url(&format!("/documents/{id}/custom-fields/{}", field.id)),
                query.return_to.as_deref(),
            )
        });
        let saved_view_href = (user.is_some()
            && field.deleted_at.is_none()
            && assignment.is_some_and(|value| value.deleted_at.is_none() && value.value.is_some()))
        .then(|| {
            crate::vault::url(&format!(
                "/saved-views/new?metadata_document_id={id}&metadata_field_id={}",
                field.id
            ))
        });
        rows.push(FieldRow {
            name: field.name,
            retired: field.deleted_at.is_some(),
            display,
            href,
            edit_href,
            saved_view_href,
        });
    }
    FieldsPanel { rows, next }
        .render()
        .map(Html)
        .map_err(|_| ApiError::internal("Could not render custom fields"))
}

#[derive(Default, Deserialize)]
struct EditorQuery {
    q: Option<String>,
    return_to: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum Operation {
    Save,
    Clear,
}

#[derive(Deserialize)]
struct EditForm {
    revision: String,
    operation: Operation,
    #[serde(default)]
    value: String,
    #[serde(default)]
    currency: String,
}

impl EditForm {
    fn revision(&self) -> ApiResult<Option<Uuid>> {
        if self.revision.is_empty() {
            Ok(None)
        } else {
            self.revision
                .parse()
                .map(Some)
                .map_err(|_| ApiError::bad_request("Invalid metadata revision"))
        }
    }

    fn typed_value(&self, kind: MetadataKind) -> ApiResult<Option<MetadataValue>> {
        if matches!(self.operation, Operation::Clear) {
            return Ok(None);
        }
        let value = self.value.clone();
        let id = || {
            self.value
                .parse()
                .map_err(|_| ApiError::bad_request("Choose a valid entry"))
        };
        Ok(Some(match kind {
            MetadataKind::Text => MetadataValue::Text(value),
            MetadataKind::LongText => MetadataValue::LongText(value),
            MetadataKind::Boolean => match self.value.as_str() {
                "true" => MetadataValue::Boolean(true),
                "false" => MetadataValue::Boolean(false),
                _ => return Err(ApiError::bad_request("Choose Yes or No")),
            },
            MetadataKind::Integer => MetadataValue::Integer(value),
            MetadataKind::Decimal => MetadataValue::Decimal(value),
            MetadataKind::Date => MetadataValue::Date(value),
            MetadataKind::Money => MetadataValue::Money {
                amount: value,
                currency: self.currency.clone(),
            },
            MetadataKind::Select => MetadataValue::Select(id()?),
            MetadataKind::Url => MetadataValue::Url(value),
            MetadataKind::DocumentLink => MetadataValue::DocumentLink(id()?),
        }))
    }
}

struct Choice {
    value: String,
    label: String,
    selected: bool,
}

impl Choice {
    fn new(value: String, label: String, selected: &str) -> Self {
        Self {
            selected: value == selected,
            value,
            label,
        }
    }
}

#[derive(Template)]
#[template(path = "metadata_document_field_edit.html")]
struct Editor {
    sidebar: Sidebar,
    document: Document,
    field: MetadataField,
    revision: String,
    value: String,
    currency: String,
    choices: Vec<Choice>,
    can_clear: bool,
    current: String,
    back: String,
    action: String,
    search: String,
    return_to: String,
    error: Option<String>,
    stale: bool,
}

async fn editor_field(
    state: &AppState,
    vault: VaultId,
    document_id: Uuid,
    field_id: Uuid,
) -> ApiResult<(Document, MetadataField)> {
    let document = document(state, vault, document_id).await?;
    if !crate::vault_context::can_edit_document(&document) {
        return Err(ApiError::forbidden("You cannot edit this document"));
    }
    let field = state
        .metadata_fields
        .find(vault, field_id)
        .await?
        .ok_or_else(|| ApiError::not_found("Field not found"))?;
    Ok((document, field))
}

async fn choices(
    state: &AppState,
    vault: VaultId,
    field: &MetadataField,
    selected: &str,
    search: &str,
) -> ApiResult<Vec<Choice>> {
    let mut choices = Vec::new();
    match field.kind {
        MetadataKind::Select => {
            let mut after = None;
            let mut live = Vec::new();
            loop {
                let options = state
                    .metadata_options
                    .list(vault, field.id, false, after, 100)
                    .await?;
                if options.is_empty() {
                    break;
                }
                after = options.last().map(|option| option.id);
                let complete = options.len() < 100;
                live.extend(options);
                if complete {
                    break;
                }
            }
            live.sort_by_key(|option| (option.position, option.id));
            choices.extend(
                live.into_iter()
                    .map(|option| Choice::new(option.id.to_string(), option.label, selected)),
            );
            if let Ok(id) = selected.parse::<Uuid>() {
                if !choices.iter().any(|choice| choice.selected) {
                    if let Some(option) = state.metadata_options.find(vault, field.id, id).await? {
                        choices.push(Choice::new(
                            id.to_string(),
                            format!("{} (retired)", option.label),
                            selected,
                        ));
                    }
                }
            }
        }
        MetadataKind::DocumentLink => {
            if search.len() > 1024 || search.split_whitespace().count() > 32 {
                return Err(ApiError::bad_request(
                    "Search supports at most 32 words and 1024 bytes",
                ));
            }
            let query = DocumentPageQuery {
                limit: 100,
                filter: folio_core::models::DocumentFilter {
                    search_query: (!search.trim().is_empty()).then(|| search.trim().to_owned()),
                    ..Default::default()
                },
                ..Default::default()
            };
            choices.extend(
                state
                    .document_repo
                    .page(vault, query)
                    .await?
                    .documents
                    .into_iter()
                    .map(|document| Choice::new(document.id.to_string(), document.title, selected)),
            );
            if let Ok(id) = selected.parse::<Uuid>() {
                if !choices.iter().any(|choice| choice.selected) {
                    if let Some(document) = state.document_repo.find_by_id(vault, id).await? {
                        choices.push(Choice::new(id.to_string(), document.title, selected));
                    } else if let Some(document) =
                        state.document_repo.find_trashed_by_id(vault, id).await?
                    {
                        choices.push(Choice::new(
                            id.to_string(),
                            format!("{} (in Trash)", document.title),
                            selected,
                        ));
                    }
                }
            }
        }
        _ => {}
    }
    Ok(choices)
}

fn current_input(assignment: Option<&DocumentMetadataValue>) -> (String, String) {
    let Some(value) = assignment.and_then(|assignment| assignment.value.as_ref()) else {
        return (String::new(), String::new());
    };
    let input = match value {
        MetadataValue::Money { amount, currency } => return (amount.clone(), currency.clone()),
        MetadataValue::Select(id) | MetadataValue::DocumentLink(id) => id.to_string(),
        MetadataValue::Boolean(value) => value.to_string(),
        MetadataValue::Text(value)
        | MetadataValue::LongText(value)
        | MetadataValue::Integer(value)
        | MetadataValue::Decimal(value)
        | MetadataValue::Date(value)
        | MetadataValue::Url(value) => value.clone(),
    };
    (input, String::new())
}

async fn render_editor(
    state: &AppState,
    vault: VaultId,
    document: Document,
    field: MetadataField,
    query: EditorQuery,
    failed: Option<(EditForm, StatusCode, String)>,
) -> ApiResult<Response> {
    let metadata = state.metadata_values.get(vault, document.id).await?;
    let assignment = metadata
        .values
        .iter()
        .find(|value| value.field_id == field.id);
    let actual_revision = metadata
        .revision
        .map_or_else(String::new, |id| id.to_string());
    let (revision, value, currency, status, error) = match failed {
        Some((draft, status, error)) => (
            draft.revision,
            draft.value,
            draft.currency,
            status,
            Some(error),
        ),
        None => {
            let (value, currency) = current_input(assignment);
            (
                actual_revision.clone(),
                value,
                currency,
                StatusCode::OK,
                None,
            )
        }
    };
    let search = query.q.unwrap_or_default();
    let choices = choices(state, vault, &field, &value, &search).await?;
    let (current, _) = display_value(state, vault, &field, assignment).await?;
    let can_clear = (field.deleted_at.is_some() || !field.required)
        && assignment
            .and_then(|assignment| assignment.value.as_ref())
            .is_some();
    let mut back = document_url(document.id, query.return_to.as_deref());
    back.push_str("#custom-fields");
    let editor_path = crate::vault::url(&format!(
        "/documents/{}/custom-fields/{}",
        document.id, field.id
    ));
    let query_string = url::form_urlencoded::Serializer::new(String::new())
        .append_pair("q", &search)
        .finish();
    let action = with_return(
        format!("{editor_path}?{query_string}"),
        query.return_to.as_deref(),
    );
    let template = Editor {
        sidebar: Sidebar::load(state, vault, Nav::None).await?,
        document,
        field,
        stale: actual_revision != revision,
        revision,
        value,
        currency,
        choices,
        can_clear,
        current,
        back,
        action,
        search,
        return_to: query.return_to.unwrap_or_default(),
        error,
    };
    let html = template
        .render()
        .map_err(|_| ApiError::internal("Could not render the custom field editor"))?;
    Ok((status, Html(html)).into_response())
}

async fn edit(
    DefaultVault(vault): DefaultVault,
    State(state): State<AppState>,
    Path((document, field)): Path<(Uuid, Uuid)>,
    Query(query): Query<EditorQuery>,
) -> ApiResult<Response> {
    let (document, field) = editor_field(&state, vault, document, field).await?;
    render_editor(&state, vault, document, field, query, None).await
}

async fn save(
    DefaultVault(vault): DefaultVault,
    State(state): State<AppState>,
    Path((document, field)): Path<(Uuid, Uuid)>,
    Query(query): Query<EditorQuery>,
    Form(draft): Form<EditForm>,
) -> ApiResult<Response> {
    let (document, field) = editor_field(&state, vault, document, field).await?;
    let revision = draft.revision()?;
    let value = match draft.typed_value(field.kind) {
        Ok(value) => value,
        Err(error) => {
            return render_editor(
                &state,
                vault,
                document,
                field,
                query,
                Some((draft, StatusCode::BAD_REQUEST, error.message().to_owned())),
            )
            .await;
        }
    };
    match state
        .metadata_values
        .patch(
            vault,
            document.id,
            revision,
            vec![MetadataChange {
                field_id: field.id,
                value,
            }],
        )
        .await
    {
        Ok(_) => Ok(Redirect::to(&format!(
            "{}#custom-fields",
            document_url(document.id, query.return_to.as_deref())
        ))
        .into_response()),
        Err(error) => {
            let (status, message) = match error {
                Error::Validation(message) => (StatusCode::UNPROCESSABLE_ENTITY, message),
                Error::Conflict(message) => (StatusCode::CONFLICT, message),
                Error::NotFound { .. } => (
                    StatusCode::NOT_FOUND,
                    "The selected field or link is unavailable".into(),
                ),
                other => return Err(other.into()),
            };
            render_editor(
                &state,
                vault,
                document,
                field,
                query,
                Some((draft, status, message)),
            )
            .await
        }
    }
}
