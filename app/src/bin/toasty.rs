//! Toasty 迁移 CLI：`cargo run -p app --bin toasty -- migration generate|apply`

use std::path::Path;

use namix::db;
use toasty_cli::{Config, MigrationConfig, ToastyCli};

#[tokio::main]
async fn main() {
    let _ = std::env::set_current_dir(env!("CARGO_MANIFEST_DIR"));
    namix::log::init();
    let url = std::env::var("DATABASE_URL").unwrap_or_else(|_| "sqlite:./storage/namix.db".into());

    let db = db::connect(&url, app::models::registry::model_set())
        .await
        .unwrap_or_else(|e| panic!("connect failed: {e}"));

    let config = Config::load_from(Path::new("Toasty.toml"))
        .unwrap_or_else(|_| Config::new().migration(MigrationConfig::new().path("database")));

    ToastyCli::with_config(db, config)
        .parse_and_run()
        .await
        .unwrap_or_else(|e| panic!("toasty failed: {e}"));
}
