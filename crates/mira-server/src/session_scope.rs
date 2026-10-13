//! Explicit HTTP ownership. Legacy callers may omit the header; browser
//! requests select a slot without mutating the process-wide active pointer.
use crate::state::AppState;
use axum::{
    extract::{FromRef, FromRequestParts},
    http::{request::Parts, StatusCode},
};

pub struct ScopedState<T = AppState>(pub T);

#[async_trait::async_trait]
impl<S: Send + Sync> FromRequestParts<S> for ScopedState
where
    AppState: FromRef<S>,
{
    type Rejection = (StatusCode, &'static str);
    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let requested = parts
            .headers
            .get("x-mira-session")
            .map(|header| {
                let id = header
                    .to_str()
                    .map_err(|_| (StatusCode::BAD_REQUEST, "invalid session ID"))?;
                if !valid_session_id(id) {
                    return Err((StatusCode::BAD_REQUEST, "invalid session ID"));
                }
                Ok(id.to_owned())
            })
            .transpose()?;
        let mut state = AppState::from_ref(state);
        if let Some(id) = requested {
            let slot = state
                .ensure_slot(&mira_core::SessionId::from(id.as_str()))
                .await
                .map_err(|_| (StatusCode::NOT_FOUND, "session not found"))?;
            state.active = std::sync::Arc::new(tokio::sync::RwLock::new(slot.id.clone()));
            state.request_slot = Some(slot);
        }
        Ok(Self(state))
    }
}

pub(crate) fn valid_session_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 128
        && id
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_')
}

#[cfg(test)]
mod tests {
    use super::*;
    struct UnavailableState;
    impl FromRef<UnavailableState> for AppState {
        fn from_ref(_: &UnavailableState) -> Self {
            panic!("invalid owners must fail before looking up any default session")
        }
    }
    #[tokio::test]
    async fn invalid_explicit_owners_never_fall_back_to_the_default_session() {
        for id in ["", "../private", "a/b", "bad session"] {
            let (mut parts, _) = axum::http::Request::builder()
                .header("x-mira-session", id)
                .body(())
                .unwrap()
                .into_parts();
            assert!(matches!(
                ScopedState::from_request_parts(&mut parts, &UnavailableState).await,
                Err((StatusCode::BAD_REQUEST, _))
            ));
        }
    }
}
