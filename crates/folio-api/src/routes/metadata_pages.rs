use askama::Template;
use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::{Html, IntoResponse, Redirect, Response},
    routing::{get, post},
    Form, Router,
};
use folio_core::models::{
    MetadataField, MetadataFieldInput, MetadataKind, MetadataOption, MetadataOptionInput, VaultId,
};
use folio_core::Error;
use serde::Deserialize;
use uuid::Uuid;

use super::view::{Nav, Sidebar};
use crate::{
    error::{ApiError, ApiResult},
    state::AppState,
    vault::{filters, DefaultVault},
};

const INDEX: &str = "/manage/fields";
const PAGE_SIZE: usize = 50;
const KINDS: [(MetadataKind, &str); 10] = [
    (MetadataKind::Text, "Text"),
    (MetadataKind::LongText, "Long text"),
    (MetadataKind::Boolean, "Yes or no"),
    (MetadataKind::Integer, "Whole number"),
    (MetadataKind::Decimal, "Decimal"),
    (MetadataKind::Date, "Date"),
    (MetadataKind::Money, "Money"),
    (MetadataKind::Select, "Select"),
    (MetadataKind::Url, "URL"),
    (MetadataKind::DocumentLink, "Document link"),
];

pub fn router() -> Router<AppState> {
    Router::new()
        .route(INDEX, get(index).post(create_field))
        .route("/manage/fields/{field}", get(detail).post(update_field))
        .route("/manage/fields/{field}/retire", post(retire_field))
        .route("/manage/fields/{field}/restore", post(restore_field))
        .route("/manage/fields/{field}/options", post(create_option))
        .route(
            "/manage/fields/{field}/options/{option}",
            post(update_option),
        )
        .route(
            "/manage/fields/{field}/options/{option}/retire",
            post(retire_option),
        )
        .route(
            "/manage/fields/{field}/options/{option}/restore",
            post(restore_option),
        )
}

#[derive(Deserialize)]
struct Page {
    after: Option<Uuid>,
}

#[derive(Clone, Deserialize)]
struct FieldForm {
    name: String,
    kind: MetadataKind,
    #[serde(default)]
    required: bool,
    #[serde(default)]
    unique: bool,
}

impl From<FieldForm> for MetadataFieldInput {
    fn from(form: FieldForm) -> Self {
        Self {
            name: form.name,
            kind: form.kind,
            required: form.required,
            unique: form.unique,
        }
    }
}

#[derive(Clone, Deserialize)]
struct OptionForm {
    label: String,
    position: u32,
}

impl From<OptionForm> for MetadataOptionInput {
    fn from(form: OptionForm) -> Self {
        Self {
            label: form.label,
            position: form.position,
        }
    }
}

struct FieldRow {
    field: MetadataField,
    kind_label: &'static str,
}

#[derive(Template)]
#[template(path = "metadata_fields.html")]
struct FieldsIndex {
    sidebar: Sidebar,
    rows: Vec<FieldRow>,
    next: Option<Uuid>,
    draft: FieldForm,
    kinds: &'static [(MetadataKind, &'static str)],
    error: Option<String>,
}

#[derive(Template)]
#[template(path = "metadata_field.html")]
struct FieldDetail {
    sidebar: Sidebar,
    field: MetadataField,
    kind_label: &'static str,
    draft: FieldForm,
    options: Vec<MetadataOption>,
    next: Option<Uuid>,
    option_draft: OptionForm,
    error: Option<String>,
}

fn render(template: impl Template, status: StatusCode) -> ApiResult<Response> {
    let html = template
        .render()
        .map_err(|_| ApiError::internal("Could not render fields"))?;
    Ok((status, Html(html)).into_response())
}

fn validation_error(error: Error) -> ApiResult<(StatusCode, String)> {
    match error {
        Error::Validation(message) => Ok((StatusCode::UNPROCESSABLE_ENTITY, message)),
        Error::Conflict(message) => Ok((StatusCode::CONFLICT, message)),
        other => Err(other.into()),
    }
}

fn take_page<Row>(mut rows: Vec<Row>, id: impl FnOnce(&Row) -> Uuid) -> (Vec<Row>, Option<Uuid>) {
    let has_more = rows.len() > PAGE_SIZE;
    rows.truncate(PAGE_SIZE);
    let next = rows.last().filter(|_| has_more).map(id);
    (rows, next)
}

fn field_url(field: Uuid) -> String {
    crate::vault_context::url(&format!("{INDEX}/{field}"))
}

async fn index(
    DefaultVault(vault): DefaultVault,
    State(state): State<AppState>,
    Query(page): Query<Page>,
) -> ApiResult<Response> {
    index_with(&state, vault, page.after, None).await
}

async fn index_with(
    state: &AppState,
    vault: VaultId,
    after: Option<Uuid>,
    failed: Option<(FieldForm, (StatusCode, String))>,
) -> ApiResult<Response> {
    let (rows, next) = take_page(
        state
            .metadata_fields
            .list(vault, true, after, PAGE_SIZE as u32 + 1)
            .await?,
        |row| row.id,
    );
    let (draft, status, error) = match failed {
        Some((draft, (status, error))) => (draft, status, Some(error)),
        None => (
            FieldForm {
                name: String::new(),
                kind: MetadataKind::Text,
                required: false,
                unique: false,
            },
            StatusCode::OK,
            None,
        ),
    };
    render(
        FieldsIndex {
            sidebar: Sidebar::load(state, vault, Nav::Manage).await?,
            rows: rows
                .into_iter()
                .map(|field| FieldRow {
                    kind_label: kind_label(field.kind),
                    field,
                })
                .collect(),
            next,
            draft,
            kinds: &KINDS,
            error,
        },
        status,
    )
}

async fn detail(
    DefaultVault(vault): DefaultVault,
    State(state): State<AppState>,
    Path(field): Path<Uuid>,
    Query(page): Query<Page>,
) -> ApiResult<Response> {
    detail_with(&state, vault, field, page.after, DetailDraft::default()).await
}

#[derive(Default)]
struct DetailDraft {
    field: Option<FieldForm>,
    option: Option<(Option<Uuid>, OptionForm)>,
    error: Option<(StatusCode, String)>,
}

async fn detail_with(
    state: &AppState,
    vault: VaultId,
    id: Uuid,
    after: Option<Uuid>,
    edit: DetailDraft,
) -> ApiResult<Response> {
    let field = state
        .metadata_fields
        .find(vault, id)
        .await?
        .ok_or_else(|| ApiError::not_found("Field not found"))?;
    let (mut options, next) = if field.kind == MetadataKind::Select {
        take_page(
            state
                .metadata_options
                .list(vault, id, true, after, PAGE_SIZE as u32 + 1)
                .await?,
            |row| row.id,
        )
    } else {
        (Vec::new(), None)
    };
    let mut option_draft = OptionForm {
        label: String::new(),
        position: 0,
    };
    if let Some((id, form)) = edit.option {
        match id {
            Some(id) => {
                if let Some(row) = options.iter_mut().find(|row| row.id == id) {
                    row.label = form.label;
                    row.position = form.position;
                }
            }
            None => option_draft = form,
        }
    }
    options.sort_by_key(|option| (option.position, option.id));
    let draft = edit.field.unwrap_or_else(|| FieldForm {
        name: field.name.clone(),
        kind: field.kind,
        required: field.required,
        unique: field.unique,
    });
    let (status, error) = edit
        .error
        .map_or((StatusCode::OK, None), |(status, error)| {
            (status, Some(error))
        });
    render(
        FieldDetail {
            sidebar: Sidebar::load(state, vault, Nav::Manage).await?,
            kind_label: kind_label(field.kind),
            field,
            draft,
            options,
            next,
            option_draft,
            error,
        },
        status,
    )
}

async fn create_field(
    DefaultVault(vault): DefaultVault,
    State(state): State<AppState>,
    Form(form): Form<FieldForm>,
) -> ApiResult<Response> {
    match state
        .metadata_fields
        .create(vault, form.clone().into())
        .await
    {
        Ok(field) => Ok(Redirect::to(&field_url(field.id)).into_response()),
        Err(error) => index_with(&state, vault, None, Some((form, validation_error(error)?))).await,
    }
}

async fn update_field(
    DefaultVault(vault): DefaultVault,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Form(form): Form<FieldForm>,
) -> ApiResult<Response> {
    match state
        .metadata_fields
        .update(vault, id, form.clone().into())
        .await
    {
        Ok(_) => Ok(Redirect::to(&field_url(id)).into_response()),
        Err(error) => {
            detail_with(
                &state,
                vault,
                id,
                None,
                DetailDraft {
                    field: Some(form),
                    error: Some(validation_error(error)?),
                    ..DetailDraft::default()
                },
            )
            .await
        }
    }
}

async fn set_field_retired(
    state: AppState,
    vault: VaultId,
    id: Uuid,
    retired: bool,
) -> ApiResult<Response> {
    match state.metadata_fields.set_deleted(vault, id, retired).await {
        Ok(_) => Ok(Redirect::to(&field_url(id)).into_response()),
        Err(error) => {
            detail_with(
                &state,
                vault,
                id,
                None,
                DetailDraft {
                    error: Some(validation_error(error)?),
                    ..DetailDraft::default()
                },
            )
            .await
        }
    }
}

async fn retire_field(
    DefaultVault(vault): DefaultVault,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> ApiResult<Response> {
    set_field_retired(state, vault, id, true).await
}

async fn restore_field(
    DefaultVault(vault): DefaultVault,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> ApiResult<Response> {
    set_field_retired(state, vault, id, false).await
}

async fn save_option(
    state: AppState,
    vault: VaultId,
    field: Uuid,
    id: Option<Uuid>,
    form: OptionForm,
) -> ApiResult<Response> {
    let result = match id {
        Some(id) => {
            state
                .metadata_options
                .update(vault, field, id, form.clone().into())
                .await
        }
        None => {
            state
                .metadata_options
                .create(vault, field, form.clone().into())
                .await
        }
    };
    match result {
        Ok(_) => Ok(Redirect::to(&field_url(field)).into_response()),
        Err(error) => {
            detail_with(
                &state,
                vault,
                field,
                None,
                DetailDraft {
                    option: Some((id, form)),
                    error: Some(validation_error(error)?),
                    ..DetailDraft::default()
                },
            )
            .await
        }
    }
}

async fn create_option(
    DefaultVault(vault): DefaultVault,
    State(state): State<AppState>,
    Path(field): Path<Uuid>,
    Form(form): Form<OptionForm>,
) -> ApiResult<Response> {
    save_option(state, vault, field, None, form).await
}

async fn update_option(
    DefaultVault(vault): DefaultVault,
    State(state): State<AppState>,
    Path((field, id)): Path<(Uuid, Uuid)>,
    Form(form): Form<OptionForm>,
) -> ApiResult<Response> {
    save_option(state, vault, field, Some(id), form).await
}

async fn set_option_retired(
    state: AppState,
    vault: VaultId,
    field: Uuid,
    id: Uuid,
    retired: bool,
) -> ApiResult<Response> {
    match state
        .metadata_options
        .set_deleted(vault, field, id, retired)
        .await
    {
        Ok(_) => Ok(Redirect::to(&field_url(field)).into_response()),
        Err(error) => {
            detail_with(
                &state,
                vault,
                field,
                None,
                DetailDraft {
                    error: Some(validation_error(error)?),
                    ..DetailDraft::default()
                },
            )
            .await
        }
    }
}

async fn retire_option(
    DefaultVault(vault): DefaultVault,
    State(state): State<AppState>,
    Path((field, id)): Path<(Uuid, Uuid)>,
) -> ApiResult<Response> {
    set_option_retired(state, vault, field, id, true).await
}

async fn restore_option(
    DefaultVault(vault): DefaultVault,
    State(state): State<AppState>,
    Path((field, id)): Path<(Uuid, Uuid)>,
) -> ApiResult<Response> {
    set_option_retired(state, vault, field, id, false).await
}

fn kind_label(kind: MetadataKind) -> &'static str {
    KINDS
        .iter()
        .find_map(|(known, label)| (*known == kind).then_some(*label))
        .unwrap_or(kind.as_str())
}
