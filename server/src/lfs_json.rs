use axum::http::{HeaderValue, header};
use axum::response::{IntoResponse, Response};
use serde::Serialize;

use crate::error::Error;

const MEDIA_TYPE: HeaderValue = HeaderValue::from_static("application/vnd.git-lfs+json");

pub struct LfsJson<T>(pub T);

impl<T: Serialize> IntoResponse for LfsJson<T> {
    fn into_response(self) -> Response {
        match serde_json::to_vec(&self.0) {
            Ok(body) => ([(header::CONTENT_TYPE, MEDIA_TYPE)], body).into_response(),
            Err(error) => Error::Serialisation(error).into_response(),
        }
    }
}
