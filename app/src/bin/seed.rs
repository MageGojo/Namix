//! 数据种子：`cargo run -p app --bin seed`

use namix::NamixToml;
use namix::db;

#[tokio::main]
async fn main() {
    let _ = std::env::set_current_dir(env!("CARGO_MANIFEST_DIR"));
    namix::log::init();
    let cfg = NamixToml::parse(include_str!("../../namix.toml"));
    let url = cfg.database.resolved_url();

    let db = db::connect(&url, app::models::registry::model_set())
        .await
        .unwrap_or_else(|e| panic!("connect failed: {e}"));
    if cfg.database.push_schema
        && let Err(e) = db.push_schema().await
        && !e.to_string().contains("already exists")
    {
        panic!("push_schema failed: {e}");
    }
    db::install(db);

    app::seeders::all::run()
        .await
        .unwrap_or_else(|e| panic!("seed failed: {e}"));
    namix::log::info!("seed complete");
}
