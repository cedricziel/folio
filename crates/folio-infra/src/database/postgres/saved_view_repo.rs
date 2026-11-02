use async_trait::async_trait;
use chrono::{DateTime, Utc};
use folio_core::error::{Error, Result};
use folio_core::models::{
    Id, SavedView, SavedViewInput, UserId, VaultId, VaultRole, ViewPresentation, ViewVisibility,
};
use folio_core::ports::{SavedViewAccess, SavedViewRepository};
use sqlx::{PgPool, Postgres, Transaction};
use std::sync::Arc;
use tokio::sync::Notify;

pub struct PostgresSavedViewRepository {
    pool: PgPool,
    wake: Arc<Notify>,
}
impl PostgresSavedViewRepository {
    pub fn new(pool: PgPool) -> Self {
        Self {
            pool,
            wake: Arc::new(Notify::new()),
        }
    }
    pub fn with_wake(mut self, wake: Arc<Notify>) -> Self {
        self.wake = wake;
        self
    }

    async fn current(
        tx: &mut Transaction<'_, Postgres>,
        vault: VaultId,
        access: &SavedViewAccess,
        id: Id,
    ) -> Result<SavedView> {
        let row = sqlx::query_as::<_, ViewRow>("SELECT * FROM saved_views WHERE vault_id = $1 AND id = $2 AND deleted_at IS NULL AND (owner_id = $3 OR visibility = 'shared') FOR UPDATE")
            .bind(vault.0).bind(id).bind(access.user.as_str())
            .fetch_optional(&mut **tx).await.map_err(storage_error)?.ok_or_else(|| missing(id))?;
        let view: SavedView = row.try_into()?;
        if !view.editable_by(&access.user, access.role) {
            return Err(missing(id));
        }
        Ok(view)
    }
}

#[derive(sqlx::FromRow)]
struct ViewRow {
    id: Id,
    vault_id: Id,
    owner_id: String,
    name: String,
    visibility: String,
    query_json: String,
    presentation: String,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
    deleted_at: Option<DateTime<Utc>>,
    created_by: Option<String>,
    updated_by: Option<String>,
}

fn storage_error(error: impl std::fmt::Display) -> Error {
    Error::Repository(error.to_string())
}
fn write_error(error: sqlx::Error) -> Error {
    if error
        .as_database_error()
        .is_some_and(|e| e.is_unique_violation())
    {
        Error::Conflict("A saved view with this name already exists".into())
    } else {
        storage_error(error)
    }
}
fn missing(id: Id) -> Error {
    Error::NotFound {
        entity_type: "Saved view",
        id: id.to_string(),
    }
}

impl TryFrom<ViewRow> for SavedView {
    type Error = Error;
    fn try_from(row: ViewRow) -> Result<Self> {
        Ok(Self {
            id: row.id,
            vault_id: VaultId(row.vault_id),
            owner_id: UserId(row.owner_id),
            name: row.name,
            visibility: match row.visibility.as_str() {
                "private" => ViewVisibility::Private,
                "shared" => ViewVisibility::Shared,
                _ => return Err(storage_error("Invalid saved-view visibility")),
            },
            query: serde_json::from_str(&row.query_json).map_err(storage_error)?,
            presentation: match row.presentation.as_str() {
                "list" => ViewPresentation::List,
                "grid" => ViewPresentation::Grid,
                _ => return Err(storage_error("Invalid saved-view presentation")),
            },
            created_at: row.created_at,
            updated_at: row.updated_at,
            deleted_at: row.deleted_at,
            created_by: row.created_by.map(UserId),
            updated_by: row.updated_by.map(UserId),
        })
    }
}

fn editable(access: &SavedViewAccess, input: &SavedViewInput) -> Result<()> {
    if input.visibility == ViewVisibility::Shared && access.role < VaultRole::Editor {
        return Err(Error::Validation(
            "Editor access is required to share a view".into(),
        ));
    }
    Ok(())
}
fn visibility(input: &SavedViewInput) -> &'static str {
    match input.visibility {
        ViewVisibility::Private => "private",
        ViewVisibility::Shared => "shared",
    }
}
fn presentation(input: &SavedViewInput) -> &'static str {
    match input.presentation {
        ViewPresentation::List => "list",
        ViewPresentation::Grid => "grid",
    }
}

#[async_trait]
impl SavedViewRepository for PostgresSavedViewRepository {
    async fn list(
        &self,
        vault: VaultId,
        access: &SavedViewAccess,
        after: Option<Id>,
        limit: u32,
    ) -> Result<Vec<SavedView>> {
        if !(1..=100).contains(&limit) {
            return Err(Error::Validation(
                "Page limit must be between 1 and 100".into(),
            ));
        }
        let rows = sqlx::query_as::<_, ViewRow>("SELECT * FROM saved_views WHERE vault_id = $1 AND deleted_at IS NULL AND (owner_id = $2 OR visibility = 'shared') AND ($3::uuid IS NULL OR id > $3) ORDER BY id LIMIT $4")
            .bind(vault.0).bind(access.user.as_str()).bind(after).bind(i64::from(limit))
            .fetch_all(&self.pool).await.map_err(storage_error)?;
        rows.into_iter().map(SavedView::try_from).collect()
    }
    async fn find(
        &self,
        vault: VaultId,
        access: &SavedViewAccess,
        id: Id,
    ) -> Result<Option<SavedView>> {
        sqlx::query_as::<_, ViewRow>("SELECT * FROM saved_views WHERE vault_id = $1 AND id = $2 AND deleted_at IS NULL AND (owner_id = $3 OR visibility = 'shared')")
            .bind(vault.0).bind(id).bind(access.user.as_str())
            .fetch_optional(&self.pool).await.map_err(storage_error)?.map(SavedView::try_from).transpose()
    }

    async fn create(
        &self,
        vault: VaultId,
        access: &SavedViewAccess,
        input: SavedViewInput,
    ) -> Result<SavedView> {
        input.validate(vault)?;
        editable(access, &input)?;
        let id = Id::new_v4();
        let now = Utc::now();
        let actor = folio_core::mutation::current_actor();
        let mut tx = self.pool.begin().await.map_err(storage_error)?;
        let row = sqlx::query_as::<_, ViewRow>("INSERT INTO saved_views (id, vault_id, owner_id, name, visibility, query_json, presentation, created_at, updated_at, created_by, updated_by) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$8,$9,$9) RETURNING *")
            .bind(id).bind(vault.0).bind(access.user.as_str()).bind(input.name.trim())
            .bind(visibility(&input)).bind(serde_json::to_string(&input.query).map_err(storage_error)?)
            .bind(presentation(&input)).bind(now).bind(actor.as_ref().map(UserId::as_str))
            .fetch_one(&mut *tx).await.map_err(write_error)?;
        let view: SavedView = row.try_into()?;
        let event = super::super::saved_view::event("saved_view.created", &view, None)?;
        super::insert_domain_event(&mut tx, &event).await?;
        tx.commit().await.map_err(storage_error)?;
        self.wake.notify_one();
        Ok(view)
    }

    async fn update(
        &self,
        vault: VaultId,
        access: &SavedViewAccess,
        id: Id,
        input: SavedViewInput,
    ) -> Result<SavedView> {
        input.validate(vault)?;
        editable(access, &input)?;
        let mut tx = self.pool.begin().await.map_err(storage_error)?;
        let current = Self::current(&mut tx, vault, access, id).await?;
        if current.name == input.name.trim()
            && current.visibility == input.visibility
            && current.presentation == input.presentation
            && serde_json::to_value(&current.query).map_err(storage_error)?
                == serde_json::to_value(&input.query).map_err(storage_error)?
        {
            tx.commit().await.map_err(storage_error)?;
            return Ok(current);
        }
        let actor = folio_core::mutation::current_actor();
        let row = sqlx::query_as::<_, ViewRow>("UPDATE saved_views SET name = $4, visibility = $5, query_json = $6, presentation = $7, updated_at = $8, updated_by = $11 WHERE vault_id = $1 AND id = $2 AND deleted_at IS NULL AND ((visibility = 'private' AND owner_id = $3) OR (visibility = 'shared' AND ((owner_id = $3 AND $9) OR $10))) RETURNING *")
            .bind(vault.0).bind(id).bind(access.user.as_str()).bind(input.name.trim())
            .bind(visibility(&input)).bind(serde_json::to_string(&input.query).map_err(storage_error)?)
            .bind(presentation(&input)).bind(Utc::now()).bind(access.role >= VaultRole::Editor).bind(access.role == VaultRole::Owner)
            .bind(actor.as_ref().map(UserId::as_str)).fetch_optional(&mut *tx).await.map_err(write_error)?.ok_or_else(|| missing(id))?;
        let view: SavedView = row.try_into()?;
        let event =
            super::super::saved_view::event("saved_view.updated", &view, Some(current.visibility))?;
        super::insert_domain_event(&mut tx, &event).await?;
        tx.commit().await.map_err(storage_error)?;
        self.wake.notify_one();
        Ok(view)
    }

    async fn delete(&self, vault: VaultId, access: &SavedViewAccess, id: Id) -> Result<()> {
        let mut tx = self.pool.begin().await.map_err(storage_error)?;
        let current = Self::current(&mut tx, vault, access, id).await?;
        let actor = folio_core::mutation::current_actor();
        let row = sqlx::query_as::<_, ViewRow>("UPDATE saved_views SET deleted_at = $4, updated_at = $4, updated_by = $7 WHERE vault_id = $1 AND id = $2 AND deleted_at IS NULL AND ((visibility = 'private' AND owner_id = $3) OR (visibility = 'shared' AND ((owner_id = $3 AND $5) OR $6))) RETURNING *")
            .bind(vault.0).bind(id).bind(access.user.as_str()).bind(Utc::now())
            .bind(access.role >= VaultRole::Editor).bind(access.role == VaultRole::Owner)
            .bind(actor.as_ref().map(UserId::as_str)).fetch_optional(&mut *tx).await.map_err(storage_error)?.ok_or_else(|| missing(id))?;
        let view: SavedView = row.try_into()?;
        let event =
            super::super::saved_view::event("saved_view.deleted", &view, Some(current.visibility))?;
        super::insert_domain_event(&mut tx, &event).await?;
        tx.commit().await.map_err(storage_error)?;
        self.wake.notify_one();
        Ok(())
    }
}
