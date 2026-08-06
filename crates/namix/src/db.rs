//! Toasty 数据库封装（Cargo features：`sqlite` / `postgresql` / `mysql` / `turso` / `dynamodb`）。
//!
//! ```ignore
//! namix::db::run(|mut db| async move {
//!     toasty::create!(User { name: "A" }).exec(&mut db).await
//! }).await?;
//!
//! let user = namix::db::optional(|mut db| async move {
//!     User::get_by_id(&mut db, 1).await
//! }).await;
//! ```

use std::future::Future;
use std::path::Path;
use std::sync::{Arc, OnceLock};

use tokio::sync::Mutex;

pub use toasty;
pub use toasty::{Db, Model, ModelSet, Result as DbResult, create, models, query, update};

static DB: OnceLock<Arc<Mutex<Db>>> = OnceLock::new();

/// 连接数据库并注册模型。
pub async fn connect(url: &str, models: ModelSet) -> DbResult<Db> {
    if let Some(parent) = sqlite_parent_dir(url) {
        let _ = std::fs::create_dir_all(parent);
    }
    toasty::Db::builder().models(models).connect(url).await
}

fn sqlite_parent_dir(url: &str) -> Option<&Path> {
    let path = url.strip_prefix("sqlite:")?;
    if path.starts_with(":memory:") {
        return None;
    }
    Path::new(path).parent()
}

/// 安装全局 Db（Boot / seed / toasty-cli 共用）。
pub fn install(db: Db) {
    if DB.set(Arc::new(Mutex::new(db))).is_err() {
        crate::log::warn!("namix::db already installed; ignoring second install");
    }
}

pub fn installed() -> bool {
    DB.get().is_some()
}

/// 取出 Db clone（共享连接池），在闭包里 `mut db` 使用。
pub async fn with<F, Fut, T>(f: F) -> DbResult<T>
where
    F: FnOnce(Db) -> Fut,
    Fut: Future<Output = DbResult<T>>,
{
    let db = clone_db()
        .await
        .expect("database not initialized — call namix::db::install(...) during Boot");
    let started = std::time::Instant::now();
    let result = f(db).await;
    tracing::debug!(
        duration_ms = started.elapsed().as_millis(),
        ok = result.is_ok(),
        "database operation"
    );
    result
}

/// [`with`] 的短名：写操作 / 需要完整错误时用。
pub async fn run<F, Fut, T>(f: F) -> DbResult<T>
where
    F: FnOnce(Db) -> Fut,
    Fut: Future<Output = DbResult<T>>,
{
    with(f).await
}

/// 读一条：成功 `Some`，找不到或出错 → `None`（≈ Eloquent `find` / `first`）。
pub async fn optional<F, Fut, T>(f: F) -> Option<T>
where
    F: FnOnce(Db) -> Fut,
    Fut: Future<Output = DbResult<T>>,
{
    with(f).await.ok()
}

/// 读列表：成功返回 Vec，失败打日志并回空（≈ 演示场景的 `all()`）。
pub async fn vec<F, Fut, T>(f: F) -> Vec<T>
where
    F: FnOnce(Db) -> Fut,
    Fut: Future<Output = DbResult<Vec<T>>>,
{
    match with(f).await {
        Ok(rows) => rows,
        Err(e) => {
            crate::log::warn!("db::vec query failed: {e}");
            Vec::new()
        }
    }
}

/// 从全局取出 Db clone（共享连接池）。
pub async fn clone_db() -> Option<Db> {
    let db = DB.get()?;
    let guard = db.lock().await;
    Some(guard.clone())
}
