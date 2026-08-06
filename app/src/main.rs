use namix::Boot;

#[tokio::main]
async fn main() {
    // 生产：NAMIX_HOME 或可执行文件旁的 dist/<ver>；开发：回退 app/
    namix::init_workdir(env!("CARGO_MANIFEST_DIR"));

    // 事件监听：注册 / 登录副作用（见 app/src/listeners）
    app::listeners::register::all();
    app::listeners::login::all();

    Boot::new("main")
        .toml(include_str!("../namix.toml"))
        .models(app::models::registry::model_set())
        // 全局：访问日志（含耗时）+ 会话水合
        .middleware(app::middleware::logger::access_log)
        .middleware(app::middleware::session::hydrate)
        .routes(app::routes::web::routes())
        .run()
        .await
        .expect("app failed");
}
