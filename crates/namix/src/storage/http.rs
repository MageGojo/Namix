//! HTTP GET/PUT for configured disk URL prefixes.

use namix_http::controller::{not_found, with_status};
use namix_http::http::StatusCode;
use namix_http::{AppError, ContentType, Request, Response, Route, Router};

use super::disks::{InstalledMeta, disk_for_url};
use super::{StorageError, Visibility};

pub fn routes() -> Router {
    let mut router = Router::new();
    for prefix in super::disks::serve_prefixes() {
        let pattern = format!("{prefix}/*path");
        let name = prefix.trim_start_matches('/').replace('/', ".");
        router = router.merge(
            Route::get(&pattern, serve_get)
                .name(format!("__namix.storage.get.{name}"))
                .register(),
        );
        router = router.merge(
            Route::put(&pattern, serve_put)
                .name(format!("__namix.storage.put.{name}"))
                .register(),
        );
    }
    router
}

async fn serve_get(req: Request) -> Response {
    let Some((_name, disk)) = disk_for_url(req.path()) else {
        return not_found();
    };
    let key = req.param("path").unwrap_or("").trim_start_matches('/');
    if key.is_empty() {
        return not_found();
    }
    if let Err(error) = authorize_get(&req, &disk, key) {
        return storage_response(&req, error);
    }
    match disk.storage.get(key) {
        Ok(Some(bytes)) => {
            let ct = ContentType::from_path(key);
            Response::new(StatusCode::OK, ct, bytes)
        }
        Ok(None) => not_found(),
        Err(error) => storage_response(&req, error),
    }
}

async fn serve_put(req: Request) -> Response {
    let Some((_name, disk)) = disk_for_url(req.path()) else {
        return not_found();
    };
    let key = req.param("path").unwrap_or("").trim_start_matches('/');
    if key.is_empty() {
        return not_found();
    }
    if let Err(error) = authorize_put(&req, &disk, key) {
        return storage_response(&req, error);
    }
    match disk.storage.put(key, req.body()) {
        Ok(()) => with_status(StatusCode::CREATED, ""),
        Err(error) => storage_response(&req, error),
    }
}

fn authorize_get(req: &Request, disk: &InstalledMeta, key: &str) -> Result<(), StorageError> {
    match signed_query(req) {
        Some((expires, signature)) => disk.storage.verify_temporary_url(key, expires, &signature),
        None if disk.visibility == Visibility::Public => Ok(()),
        None => Err(StorageError::NotFound),
    }
}

fn authorize_put(req: &Request, disk: &InstalledMeta, key: &str) -> Result<(), StorageError> {
    let (expires, signature) = signed_query(req).ok_or(StorageError::NotFound)?;
    disk.storage
        .verify_temporary_upload_url(key, expires, &signature)
}

fn signed_query(req: &Request) -> Option<(u64, String)> {
    let expires = req.query("expires")?.parse().ok()?;
    let signature = req.query("signature").filter(|value| !value.is_empty())?;
    Some((expires, signature))
}

fn storage_response(req: &Request, error: StorageError) -> Response {
    let app: AppError = error.into();
    app.into_response_for(req)
}
