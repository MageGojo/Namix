//! Durable queue worker: `nx work` / `cargo run -p app --bin work`

#[tokio::main]
async fn main() {
    namix::init_workdir(env!("CARGO_MANIFEST_DIR"));
    app::facades::install();
    app::listeners::register::all();
    app::listeners::login::all();

    namix::Boot::new("main")
        .toml(include_str!("../../namix.toml"))
        .models(app::models::registry::model_set())
        .work()
        .await
        .expect("queue worker failed");
}
