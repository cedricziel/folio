//! Server-rendered pages for managing ingestion sources

use askama::Template;
use axum::{
    extract::{Path, State},
    response::{Html, IntoResponse, Redirect, Response},
    routing::{get, post},
    Form, Router,
};
use folio_core::models::VaultId;
use serde::Deserialize;
use uuid::Uuid;

use folio_core::models::{
    EmailConfig, FolderAfterImport, FolderConfig, IngestionSourceDef, IngestionSourceInput,
    MailAfterImport, MailBody, MailSecurity, SourceConfig, SourceOrigin, REDACTED,
};

use super::sources::{all_sources, find, prepare_input, require_managed, run_test};
use super::view::{Nav, Sidebar};
use crate::error::{ApiError, ApiResult};
use crate::state::AppState;
use crate::vault::DefaultVault;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/settings/sources", get(list_page).post(create))
        .route("/settings/sources/new", get(new_page))
        .route("/settings/sources/{id}", get(edit_page).post(update))
        .route("/settings/sources/{id}/toggle", post(toggle))
        .route("/settings/sources/{id}/delete", post(delete))
        .route("/settings/sources/{id}/test", post(test_saved))
}

struct Notice {
    danger: bool,
    text: String,
}

struct Row {
    id: String,
    name: String,
    kind: &'static str,
    summary: String,
    enabled: bool,
    read_only: bool,
    last_run: Option<String>,
    last_result: Option<String>,
    error: Option<String>,
}

#[derive(Template)]
#[template(path = "sources.html")]
struct ListTemplate {
    sidebar: Sidebar,
    title: String,
    rows: Vec<Row>,
    notice: Option<Notice>,
}

fn summary(def: &IngestionSourceDef) -> String {
    match &def.config {
        SourceConfig::Folder(f) => f.path.clone(),
        SourceConfig::Email(e) => format!("{}@{}:{}", e.username, e.host, e.port),
    }
}

async fn rows(DefaultVault(vault): DefaultVault, state: &AppState) -> ApiResult<Vec<Row>> {
    let mut rows = Vec::new();
    for def in all_sources(DefaultVault(vault), state).await? {
        let status = state.ingestion.managed.status(def.id).await?;
        rows.push(Row {
            id: def.id.to_string(),
            kind: match def.config {
                SourceConfig::Folder(_) => "Folder",
                SourceConfig::Email(_) => "Email",
            },
            summary: summary(&def),
            read_only: def.origin == SourceOrigin::Config,
            enabled: def.enabled,
            last_run: status
                .as_ref()
                .and_then(|s| s.last_run_at)
                .map(|t| t.format("%Y-%m-%d %H:%M UTC").to_string()),
            last_result: status
                .as_ref()
                .filter(|s| s.last_run_at.is_some())
                .map(|s| {
                    format!(
                        "{} imported, {} skipped, {} failed",
                        s.last_imported, s.last_skipped, s.last_failed
                    )
                }),
            error: status.and_then(|s| s.last_error),
            name: def.name,
        });
    }
    Ok(rows)
}

async fn list_page(
    DefaultVault(vault): DefaultVault,
    State(state): State<AppState>,
) -> ApiResult<Html<String>> {
    render(ListTemplate {
        sidebar: Sidebar::load(&state, vault, Nav::Settings).await?,
        title: "Sources - Folio".into(),
        rows: rows(DefaultVault(vault), &state).await?,
        notice: None,
    })
}

async fn test_saved(
    DefaultVault(vault): DefaultVault,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> ApiResult<Html<String>> {
    let def = find(vault, &state, id).await?;
    let result = run_test(&state, &def).await;
    render(ListTemplate {
        sidebar: Sidebar::load(&state, vault, Nav::Settings).await?,
        title: "Sources - Folio".into(),
        rows: rows(DefaultVault(vault), &state).await?,
        notice: Some(Notice {
            danger: !result.ok,
            text: format!("{}: {}", def.name, result.message),
        }),
    })
}

/// Form fields for both source kinds; the unused kind's fields stay empty
#[derive(Default, Deserialize, Clone)]
#[serde(default)]
struct FormValues {
    kind: String,
    name: String,
    enabled: Option<String>,
    // folder
    path: String,
    recursive: Option<String>,
    subfolders_as_tags: Option<String>,
    folder_after_import: String,
    settle_secs: String,
    rescan_secs: String,
    // email
    host: String,
    port: String,
    security: String,
    accept_invalid_certs: Option<String>,
    username: String,
    password: String,
    mailboxes: String,
    poll_interval_secs: String,
    criteria: String,
    allowed_content_types: String,
    mail_body: String,
    mail_after_import: String,
    move_to: String,
    /// Set by the "Test connection" button
    test: Option<String>,
}

impl FormValues {
    fn blank(kind: &str) -> Self {
        let config = if kind == "email" {
            SourceConfig::Email(
                serde_json::from_value(serde_json::json!({"host": "", "username": ""}))
                    .expect("mail defaults deserialize"),
            )
        } else {
            SourceConfig::Folder(
                serde_json::from_value(serde_json::json!({"path": ""}))
                    .expect("folder defaults deserialize"),
            )
        };
        Self::from_config(&config, "", true)
    }

    fn from_config(config: &SourceConfig, name: &str, enabled: bool) -> Self {
        let on = |b: bool| b.then(|| "on".to_string());
        let mut v = Self {
            name: name.to_string(),
            enabled: on(enabled),
            ..Default::default()
        };
        match config {
            SourceConfig::Folder(f) => {
                v.kind = "folder".into();
                v.path = f.path.clone();
                v.recursive = on(f.recursive);
                v.subfolders_as_tags = on(f.subfolders_as_tags);
                v.folder_after_import = match f.after_import {
                    FolderAfterImport::Delete => "delete",
                    FolderAfterImport::Move => "move",
                }
                .into();
                v.settle_secs = f.settle_secs.to_string();
                v.rescan_secs = f.rescan_secs.to_string();
            }
            SourceConfig::Email(e) => {
                v.kind = "email".into();
                v.host = e.host.clone();
                v.port = e.port.to_string();
                v.security = match e.security {
                    MailSecurity::Tls => "tls",
                    MailSecurity::Plain => "plain",
                }
                .into();
                v.accept_invalid_certs = on(e.accept_invalid_certs);
                v.username = e.username.clone();
                v.mailboxes = e.mailboxes.join("\n");
                v.poll_interval_secs = e.poll_interval_secs.to_string();
                v.criteria = e.criteria.clone();
                v.allowed_content_types = e.allowed_content_types.join("\n");
                v.mail_body = match e.mail_body {
                    MailBody::None => "none",
                    MailBody::Eml => "eml",
                }
                .into();
                v.mail_after_import = match e.after_import {
                    MailAfterImport::MarkRead => "mark_read",
                    MailAfterImport::Move => "move",
                    MailAfterImport::Delete => "delete",
                }
                .into();
                v.move_to = e.move_to.clone().unwrap_or_default();
                // The stored password never reaches the page; the mask stands for it.
                if e.password.is_some() {
                    v.password = REDACTED.into();
                }
            }
        }
        v
    }

    fn into_input(self) -> Result<IngestionSourceInput, String> {
        let number = |s: &str, what: &str| -> Result<u64, String> {
            s.trim()
                .parse()
                .map_err(|_| format!("{what} must be a whole number."))
        };
        let lines = |s: &str| -> Vec<String> {
            s.split(['\n', ','])
                .map(str::trim)
                .filter(|l| !l.is_empty())
                .map(String::from)
                .collect()
        };
        let config = match self.kind.as_str() {
            "folder" => SourceConfig::Folder(FolderConfig {
                path: self.path.trim().to_string(),
                recursive: self.recursive.is_some(),
                subfolders_as_tags: self.subfolders_as_tags.is_some(),
                after_import: if self.folder_after_import == "delete" {
                    FolderAfterImport::Delete
                } else {
                    FolderAfterImport::Move
                },
                settle_secs: number(&self.settle_secs, "Settle time")?,
                rescan_secs: number(&self.rescan_secs, "Rescan interval")?,
            }),
            "email" => SourceConfig::Email(EmailConfig {
                host: self.host.trim().to_string(),
                port: u16::try_from(number(&self.port, "Port")?)
                    .map_err(|_| "Port is out of range.".to_string())?,
                security: if self.security == "plain" {
                    MailSecurity::Plain
                } else {
                    MailSecurity::Tls
                },
                accept_invalid_certs: self.accept_invalid_certs.is_some(),
                username: self.username.trim().to_string(),
                password: Some(self.password).filter(|p| !p.is_empty()),
                password_file: None,
                mailboxes: lines(&self.mailboxes),
                poll_interval_secs: number(&self.poll_interval_secs, "Poll interval")?,
                criteria: self.criteria.trim().to_string(),
                allowed_content_types: lines(&self.allowed_content_types),
                mail_body: if self.mail_body == "eml" {
                    MailBody::Eml
                } else {
                    MailBody::None
                },
                after_import: match self.mail_after_import.as_str() {
                    "move" => MailAfterImport::Move,
                    "delete" => MailAfterImport::Delete,
                    _ => MailAfterImport::MarkRead,
                },
                move_to: Some(self.move_to.trim().to_string()).filter(|m| !m.is_empty()),
            }),
            _ => return Err("Choose a source type.".to_string()),
        };
        Ok(IngestionSourceInput {
            name: self.name.trim().to_string(),
            enabled: self.enabled.is_some(),
            config,
        })
    }
}

#[derive(Template)]
#[template(path = "source_form.html")]
struct FormTemplate {
    sidebar: Sidebar,
    title: String,
    source_id: Option<String>,
    values: FormValues,
    notice: Option<Notice>,
}

async fn form(
    vault: VaultId,
    state: &AppState,
    source_id: Option<String>,
    values: FormValues,
    notice: Option<Notice>,
) -> ApiResult<Html<String>> {
    render(FormTemplate {
        sidebar: Sidebar::load(state, vault, Nav::Settings).await?,
        title: format!(
            "{} - Folio",
            if source_id.is_some() {
                "Edit source"
            } else {
                "New source"
            }
        ),
        source_id,
        values,
        notice,
    })
}

fn danger(text: impl Into<String>) -> Option<Notice> {
    Some(Notice {
        danger: true,
        text: text.into(),
    })
}

#[derive(Deserialize)]
struct NewQuery {
    kind: Option<String>,
}

async fn new_page(
    DefaultVault(vault): DefaultVault,
    State(state): State<AppState>,
    axum::extract::Query(q): axum::extract::Query<NewQuery>,
) -> ApiResult<Html<String>> {
    form(
        vault,
        &state,
        None,
        FormValues::blank(q.kind.as_deref().unwrap_or("folder")),
        None,
    )
    .await
}

async fn edit_page(
    DefaultVault(vault): DefaultVault,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> ApiResult<Html<String>> {
    let def = find(vault, &state, id).await?;
    require_managed(&def)?;
    form(
        vault,
        &state,
        Some(id.to_string()),
        FormValues::from_config(&def.config, &def.name, def.enabled),
        None,
    )
    .await
}

/// Run the connection check for what is in the form, without saving
async fn try_connection(
    state: &AppState,
    values: FormValues,
    existing: Option<&IngestionSourceDef>,
) -> Option<Notice> {
    let mut input = match values.into_input() {
        Ok(input) => input,
        Err(message) => return danger(message),
    };
    if let Err(e) = prepare_input(&mut input, existing) {
        return danger(e.to_string());
    }
    let def = IngestionSourceDef {
        vault_id: existing.and_then(|d| d.vault_id),
        id: existing.map(|d| d.id).unwrap_or_else(Uuid::new_v4),
        name: input.name,
        enabled: true,
        origin: SourceOrigin::Managed,
        config: input.config,
    };
    let result = run_test(state, &def).await;
    Some(Notice {
        danger: !result.ok,
        text: result.message,
    })
}

async fn create(
    DefaultVault(vault): DefaultVault,
    State(state): State<AppState>,
    Form(values): Form<FormValues>,
) -> ApiResult<Response> {
    if values.test.is_some() {
        let notice = try_connection(&state, values.clone(), None).await;
        return Ok(form(vault, &state, None, values, notice)
            .await?
            .into_response());
    }
    let saved = match values.clone().into_input() {
        Ok(mut input) => match prepare_input(&mut input, None) {
            Ok(()) => state
                .ingestion
                .managed
                .create(vault, input)
                .await
                .map_err(|e| e.to_string()),
            Err(e) => Err(e.to_string()),
        },
        Err(message) => Err(message),
    };
    match saved {
        Ok(_) => Ok(Redirect::to("/settings/sources").into_response()),
        Err(message) => Ok(form(vault, &state, None, values, danger(message))
            .await?
            .into_response()),
    }
}

async fn update(
    DefaultVault(vault): DefaultVault,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Form(values): Form<FormValues>,
) -> ApiResult<Response> {
    let existing = find(vault, &state, id).await?;
    require_managed(&existing)?;
    if values.test.is_some() {
        let notice = try_connection(&state, values.clone(), Some(&existing)).await;
        return Ok(form(vault, &state, Some(id.to_string()), values, notice)
            .await?
            .into_response());
    }
    let saved = match values.clone().into_input() {
        Ok(mut input) => match prepare_input(&mut input, Some(&existing)) {
            Ok(()) => state
                .ingestion
                .managed
                .update(vault, id, input)
                .await
                .map_err(|e| e.to_string()),
            Err(e) => Err(e.to_string()),
        },
        Err(message) => Err(message),
    };
    match saved {
        Ok(_) => Ok(Redirect::to("/settings/sources").into_response()),
        Err(message) => Ok(
            form(vault, &state, Some(id.to_string()), values, danger(message))
                .await?
                .into_response(),
        ),
    }
}

async fn toggle(
    DefaultVault(vault): DefaultVault,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> ApiResult<Redirect> {
    let def = find(vault, &state, id).await?;
    require_managed(&def)?;
    state
        .ingestion
        .managed
        .update(
            vault,
            id,
            IngestionSourceInput {
                name: def.name,
                enabled: !def.enabled,
                config: def.config,
            },
        )
        .await?;
    Ok(Redirect::to("/settings/sources"))
}

async fn delete(
    DefaultVault(vault): DefaultVault,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> ApiResult<Redirect> {
    let def = find(vault, &state, id).await?;
    require_managed(&def)?;
    state.ingestion.managed.delete(vault, id).await?;
    Ok(Redirect::to("/settings/sources"))
}

fn render<T: Template>(template: T) -> ApiResult<Html<String>> {
    template
        .render()
        .map(Html)
        .map_err(|e| ApiError::internal(format!("Failed to render page: {e}")))
}
