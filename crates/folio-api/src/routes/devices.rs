//! Apple devices registered for push notifications (JSON API)

use std::sync::Arc;

use axum::{
    extract::{Path, State},
    http::StatusCode,
    routing::{get, post, put},
    Extension, Json, Router,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use folio_auth::AuthContext;
use folio_core::models::{
    valid_push_token, DevicePlatform, DeviceRegistration, NotificationPreferences, PushDevice,
    PushEnvironment, UserId, VaultSelection,
};
use folio_core::ports::PushDeviceRepository;
use folio_core::services::notifications::{NotificationSender, TestRefusal};
use folio_infra::push::ConfiguredPush;

use crate::error::{ApiError, ApiResult};
use crate::state::AppState;
use crate::vault::Uploader;

/// Each selected vault costs an access lookup per registration.
const MAX_SELECTED_VAULTS: usize = 100;

/// What push notification routes need
#[derive(Clone)]
pub struct PushState {
    pub devices: Arc<dyn PushDeviceRepository>,
    pub sender: Arc<NotificationSender>,
    pub settings: Arc<ConfiguredPush>,
}

pub(crate) fn router() -> Router<AppState> {
    Router::new()
        .route("/devices", get(list_devices))
        .route(
            "/devices/{installation_id}",
            put(register_device).delete(delete_device),
        )
        .route("/devices/{installation_id}/test", post(test_device))
}

#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct DeviceRegistrationInput {
    pub platform: DevicePlatform,
    pub environment: PushEnvironment,
    /// BCP 47 language tag the alerts are written in, such as `de-DE`
    pub locale: String,
    /// APNs device token for alerts, hexadecimal; absent keeps the stored one
    pub alert_token: Option<String>,
    /// WidgetKit push token, hexadecimal; absent keeps the stored one
    pub widget_token: Option<String>,
    #[serde(default)]
    pub preferences: NotificationPreferences,
}

#[derive(Serialize, ToSchema)]
pub struct DeviceView {
    pub installation_id: String,
    pub platform: DevicePlatform,
    pub environment: PushEnvironment,
    pub locale: String,
    pub preferences: NotificationPreferences,
    pub has_alert_token: bool,
    pub has_widget_token: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub last_delivered_at: Option<DateTime<Utc>>,
}

impl From<PushDevice> for DeviceView {
    fn from(device: PushDevice) -> Self {
        Self {
            installation_id: device.installation_id,
            platform: device.platform,
            environment: device.environment,
            locale: device.locale,
            preferences: device.preferences,
            has_alert_token: device.alert_token.is_some(),
            has_widget_token: device.widget_token.is_some(),
            created_at: device.created_at,
            updated_at: device.updated_at,
            last_delivered_at: device.last_delivered_at,
        }
    }
}

#[derive(Serialize, ToSchema)]
pub struct DeviceRegistrationResponse {
    pub device: DeviceView,
    /// Whether this server can deliver notifications right now
    pub push_available: bool,
}

#[derive(Serialize, ToSchema)]
pub struct DeviceList {
    pub devices: Vec<DeviceView>,
    /// Whether this server can deliver notifications right now
    pub push_available: bool,
}

fn push(state: &AppState) -> ApiResult<&PushState> {
    state
        .push
        .as_ref()
        .ok_or_else(|| ApiError::new(StatusCode::NOT_FOUND, "Push notifications are not set up"))
}

fn signed_in(user: Option<UserId>) -> ApiResult<UserId> {
    user.ok_or_else(|| ApiError::new(StatusCode::UNAUTHORIZED, "Sign in to manage devices"))
}

fn unprocessable(message: impl Into<String>) -> ApiError {
    ApiError::new(StatusCode::UNPROCESSABLE_ENTITY, message)
}

fn valid_installation_id(id: &str) -> bool {
    (1..=128).contains(&id.len())
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

fn valid_locale(locale: &str) -> bool {
    (2..=35).contains(&locale.len())
        && locale
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

#[utoipa::path(put, path = "/api/devices/{installation_id}", operation_id = "registerDevice", tag = "devices",
    params(("installation_id" = String, Path, description = "Stable ID of the app installation")),
    request_body = DeviceRegistrationInput,
    responses(
        (status = 201, description = "Device registered", body = DeviceRegistrationResponse),
        (status = 200, description = "Device updated", body = DeviceRegistrationResponse),
        (status = 403, description = "Only apps signed in through OAuth can register", body = crate::error::ErrorResponse),
        (status = 422, description = "Invalid platform, environment, token, locale or vault", body = crate::error::ErrorResponse),
    ))]
pub(crate) async fn register_device(
    State(state): State<AppState>,
    auth: Option<Extension<AuthContext>>,
    Path(installation_id): Path<String>,
    Json(input): Json<DeviceRegistrationInput>,
) -> ApiResult<(StatusCode, Json<DeviceRegistrationResponse>)> {
    let push = push(&state)?;
    let Some(Extension(auth)) = auth
        .filter(|Extension(auth)| auth.personal_token_id.is_none() && !auth.grant_id.is_empty())
    else {
        return Err(ApiError::forbidden(
            "Only an app signed in through OAuth can register for notifications",
        ));
    };
    if !valid_installation_id(&installation_id) {
        return Err(unprocessable("Invalid installation ID"));
    }
    if !valid_locale(&input.locale) {
        return Err(unprocessable("Invalid locale"));
    }
    for token in [&input.alert_token, &input.widget_token]
        .into_iter()
        .flatten()
    {
        if !valid_push_token(token) {
            return Err(unprocessable("Push tokens must be hexadecimal"));
        }
    }
    let user = UserId(auth.user_id.clone());
    let mut input = input;
    if let VaultSelection::Only(vaults) = &mut input.preferences.vaults {
        vaults.sort();
        vaults.dedup();
        if vaults.len() > MAX_SELECTED_VAULTS {
            return Err(unprocessable(format!(
                "Too many vaults selected; choose at most {MAX_SELECTED_VAULTS} or all"
            )));
        }
        for vault in vaults.iter() {
            if state.access.effective_role(&user, *vault).await?.is_none() {
                return Err(unprocessable(format!("Vault {vault} is not accessible")));
            }
        }
    }
    let (device, created) = push
        .devices
        .upsert(DeviceRegistration {
            user_id: user,
            installation_id,
            client_id: auth.client_id.clone(),
            grant_id: auth.grant_id.clone(),
            platform: input.platform,
            environment: input.environment,
            locale: input.locale,
            preferences: input.preferences,
            alert_token: input.alert_token.map(|t| t.to_ascii_lowercase()),
            widget_token: input.widget_token.map(|t| t.to_ascii_lowercase()),
        })
        .await?;
    let status = if created {
        StatusCode::CREATED
    } else {
        StatusCode::OK
    };
    Ok((
        status,
        Json(DeviceRegistrationResponse {
            device: device.into(),
            push_available: push.sender.available().await,
        }),
    ))
}

#[utoipa::path(get, path = "/api/devices", operation_id = "listDevices", tag = "devices",
    responses((status = 200, description = "The caller's devices, without tokens", body = DeviceList)))]
pub(crate) async fn list_devices(
    State(state): State<AppState>,
    Uploader(user): Uploader,
) -> ApiResult<Json<DeviceList>> {
    let push = push(&state)?;
    let user = signed_in(user)?;
    let devices = push.devices.list_for_user(&user).await?;
    Ok(Json(DeviceList {
        devices: devices.into_iter().map(Into::into).collect(),
        push_available: push.sender.available().await,
    }))
}

#[utoipa::path(delete, path = "/api/devices/{installation_id}", operation_id = "deleteDevice", tag = "devices",
    params(("installation_id" = String, Path, description = "Stable ID of the app installation")),
    responses((status = 204, description = "The device is not registered (any more)")))]
pub(crate) async fn delete_device(
    State(state): State<AppState>,
    Uploader(user): Uploader,
    Path(installation_id): Path<String>,
) -> ApiResult<StatusCode> {
    let push = push(&state)?;
    let user = signed_in(user)?;
    push.devices.delete(&user, &installation_id).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[utoipa::path(post, path = "/api/devices/{installation_id}/test", operation_id = "sendTestNotification", tag = "devices",
    params(("installation_id" = String, Path, description = "Stable ID of the app installation")),
    responses(
        (status = 202, description = "A test notification is on its way"),
        (status = 404, description = "No such device", body = crate::error::ErrorResponse),
        (status = 409, description = "Push notifications are not available on this server", body = crate::error::ErrorResponse),
        (status = 429, description = "A test was sent less than a minute ago", body = crate::error::ErrorResponse),
    ))]
pub(crate) async fn test_device(
    State(state): State<AppState>,
    Uploader(user): Uploader,
    Path(installation_id): Path<String>,
) -> ApiResult<StatusCode> {
    let push = push(&state)?;
    let user = signed_in(user)?;
    match push
        .sender
        .send_test(&user, &installation_id, Utc::now())
        .await
    {
        Ok(()) => Ok(StatusCode::ACCEPTED),
        Err(TestRefusal::NotFound) => Err(ApiError::not_found("Device not found")),
        Err(TestRefusal::NoToken) => Err(ApiError::new(
            StatusCode::CONFLICT,
            "This device has no notification token",
        )),
        Err(TestRefusal::Unavailable) => Err(ApiError::new(
            StatusCode::CONFLICT,
            "Push notifications are not configured on this server",
        )),
        Err(TestRefusal::TooSoon) => Err(ApiError::new(
            StatusCode::TOO_MANY_REQUESTS,
            "Wait a minute before sending another test",
        )),
    }
}
