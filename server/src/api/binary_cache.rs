//! Nix Binary Cache server.
//!
//! This module implements the Nix Binary Cache API.
//!
//! The implementation is based on the specifications at <https://github.com/fzakaria/nix-http-binary-cache-api-spec>.

use std::path::PathBuf;

use axum::{
    body::StreamBody,
    extract::{Extension, Path},
    http::StatusCode,
    response::{IntoResponse, Redirect, Response},
    routing::get,
    Router,
};
use serde::Serialize;
use tokio_util::io::ReaderStream;
use tracing::instrument;

use crate::database::AtticDatabase;
use crate::error::{ServerError, ServerResult};
use crate::narinfo::NarInfo;
use crate::nix_manifest;
use crate::storage::Download;
use crate::{RequestState, State};
use attic::cache::CacheName;
use attic::mime;
use attic::nix_store::StorePathHash;

/// Nix cache information.
///
/// An example of a correct response is as follows:
///
/// ```text
/// StoreDir: /nix/store
/// WantMassQuery: 1
/// Priority: 40
/// ```
#[derive(Debug, Clone, Serialize)]
struct NixCacheInfo {
    /// Whether this binary cache supports bulk queries.
    #[serde(rename = "WantMassQuery")]
    want_mass_query: bool,

    /// The Nix store path this binary cache uses.
    #[serde(rename = "StoreDir")]
    store_dir: PathBuf,

    /// The priority of the binary cache.
    ///
    /// A lower number denotes a higher priority.
    /// <https://cache.nixos.org> has a priority of 40.
    #[serde(rename = "Priority")]
    priority: i32,
}

impl IntoResponse for NixCacheInfo {
    fn into_response(self) -> Response {
        match nix_manifest::to_string(&self) {
            Ok(body) => Response::builder()
                .status(StatusCode::OK)
                .header("Content-Type", mime::NIX_CACHE_INFO)
                .body(body)
                .unwrap()
                .into_response(),
            Err(e) => e.into_response(),
        }
    }
}

/// Gets information on a cache.
#[instrument(skip_all, fields(cache_name))]
async fn get_nix_cache_info(
    Extension(state): Extension<State>,
    Extension(req_state): Extension<RequestState>,
    Path(cache_name): Path<CacheName>,
) -> ServerResult<NixCacheInfo> {
    let database = state.database().await?;
    let cache = req_state
        .auth
        .auth_cache(database, &cache_name, |cache, permission| {
            permission.require_pull()?;
            Ok(cache)
        })
        .await?;

    let info = NixCacheInfo {
        want_mass_query: true,
        store_dir: cache.store_dir.into(),
        priority: cache.priority,
    };

    Ok(info)
}

/// Gets various information on a store path hash.
///
/// `/:cache/:path`, which may be one of
/// - GET `/:cache/{storePathHash}.narinfo`
/// - HEAD `/:cache/{storePathHash}.narinfo`
/// - GET `/:cache/{storePathHash}.ls` (not implemented)
#[instrument(skip_all, fields(cache_name, path))]
async fn get_store_path_info(
    Extension(state): Extension<State>,
    Extension(req_state): Extension<RequestState>,
    Path((cache_name, path)): Path<(CacheName, String)>,
) -> ServerResult<NarInfo> {
    let components: Vec<&str> = path.splitn(2, '.').collect();

    if components.len() != 2 {
        return Err(ServerError::NotFound);
    }

    // TODO: Other endpoints
    if components[1] != "narinfo" {
        return Err(ServerError::NotFound);
    }

    let store_path_hash = StorePathHash::new(components[0].to_string())?;

    tracing::debug!(
        "Received request for {}.narinfo in {:?}",
        store_path_hash.as_str(),
        cache_name
    );

    let (object, cache, nar) = state
        .database()
        .await?
        .find_object_by_store_path_hash(&cache_name, &store_path_hash)
        .await?;

    let permission = req_state
        .auth
        .get_permission_for_cache(&cache_name, cache.is_public);
    permission.require_pull()?;

    let mut narinfo = object.to_nar_info(&nar)?;

    if narinfo.signature().is_none() {
        let keypair = cache.keypair()?;
        narinfo.sign(&keypair);
    }

    Ok(narinfo)
}

/// Gets a NAR.
///
/// - GET `:cache/nar/{storePathHash}.nar`
///
/// Here we use the store path hash not the NAR hash or file hash
/// for better logging. In reality, the files are deduplicated by
/// content-addressing.
#[instrument(skip_all, fields(cache_name, path))]
async fn get_nar(
    Extension(state): Extension<State>,
    Extension(req_state): Extension<RequestState>,
    Path((cache_name, path)): Path<(CacheName, String)>,
) -> ServerResult<Response> {
    let components: Vec<&str> = path.splitn(2, '.').collect();

    if components.len() != 2 {
        return Err(ServerError::NotFound);
    }

    if components[1] != "nar" {
        return Err(ServerError::NotFound);
    }

    let store_path_hash = StorePathHash::new(components[0].to_string())?;

    tracing::debug!(
        "Received request for {}.nar in {:?}",
        store_path_hash.as_str(),
        cache_name
    );

    let database = state.database().await?;

    let (object, cache, nar) = database
        .find_object_by_store_path_hash(&cache_name, &store_path_hash)
        .await?;

    let permission = req_state
        .auth
        .get_permission_for_cache(&cache_name, cache.is_public);
    permission.require_pull()?;

    database.bump_object_last_accessed(object.id).await?;

    let remote_file = nar.remote_file.0;
    let backend = state.storage().await?;
    match backend.download_file_db(&remote_file).await? {
        Download::Redirect(uri) => Ok(Redirect::temporary(&uri).into_response()),
        Download::Stream(stream) => {
            let stream = ReaderStream::new(stream);
            let body = StreamBody::new(stream);

            Ok(body.into_response())
        }
    }
}

pub fn get_router() -> Router {
    Router::new()
        .route("/:cache/nix-cache-info", get(get_nix_cache_info))
        .route("/:cache/:path", get(get_store_path_info))
        .route("/:cache/nar/:path", get(get_nar))
}