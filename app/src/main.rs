use namix::Boot;

#[tokio::main]
async fn main() {
    // 生产：NAMIX_HOME 或可执行文件旁的 dist/<ver>；开发：回退 app/
    namix::init_workdir(env!("CARGO_MANIFEST_DIR"));

    app::facades::install();
    app::listeners::register::all();
    app::listeners::login::all();

    Boot::new("main")
        .toml(include_str!("../namix.toml"))
        .models(app::models::registry::model_set())
        .document(
            namix::Document::new()
                .head(r#"<meta name="color-scheme" content="light dark">"#)
                .template_file("src/views/layouts/app.html")
                .expect("document template src/views/layouts/app.html"),
        )
        // 全局：访问日志（含耗时）+ 会话水合 + 文档壳暗亮色
        .middleware(app::middleware::logger::access_log)
        .middleware(app::middleware::session::hydrate)
        .middleware(app::middleware::document::apply)
        .routes(app::routes::web::routes())
        .run()
        .await
        .expect("app failed");
}
