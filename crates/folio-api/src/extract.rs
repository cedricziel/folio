//! Extractors for the JSON API whose rejections are `ApiError`s.
//!
//! The stock `Json`, `Path` and `Query` extractors reject with plain text,
//! which contradicts the `ErrorResponse` the OpenAPI spec promises.

use axum::{
    extract::{
        rejection::{JsonRejection, PathRejection, QueryRejection},
        FromRequest, FromRequestParts, Path, Query, Request,
    },
    http::request::Parts,
    Json,
};
use serde::de::DeserializeOwned;

use crate::error::ApiError;

macro_rules! rejection_to_api_error {
    ($($rejection:ty),*) => {$(
        impl From<$rejection> for ApiError {
            fn from(rejection: $rejection) -> Self {
                ApiError::new(rejection.status(), rejection.body_text())
            }
        }
    )*};
}

rejection_to_api_error!(JsonRejection, PathRejection, QueryRejection);

/// `Json` that rejects with a JSON `ErrorResponse`
pub struct ApiJson<T>(pub T);

impl<S, T> FromRequest<S> for ApiJson<T>
where
    S: Send + Sync,
    T: DeserializeOwned,
{
    type Rejection = ApiError;

    async fn from_request(req: Request, state: &S) -> Result<Self, Self::Rejection> {
        Ok(Self(Json::<T>::from_request(req, state).await?.0))
    }
}

/// `Path` that rejects with a JSON `ErrorResponse`
pub struct ApiPath<T>(pub T);

impl<S, T> FromRequestParts<S> for ApiPath<T>
where
    S: Send + Sync,
    T: DeserializeOwned + Send,
{
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        Ok(Self(Path::<T>::from_request_parts(parts, state).await?.0))
    }
}

/// `Query` that rejects with a JSON `ErrorResponse`
pub struct ApiQuery<T>(pub T);

impl<S, T> FromRequestParts<S> for ApiQuery<T>
where
    S: Send + Sync,
    T: DeserializeOwned,
{
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        Ok(Self(Query::<T>::from_request_parts(parts, state).await?.0))
    }
}
