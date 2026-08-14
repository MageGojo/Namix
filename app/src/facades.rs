//! Shared facades installed by the HTTP bin and `nx work`.

pub fn install() {
    namix::access::install(
        namix::Access::new()
            .role("admin", &["*"])
            .role("user", &["posts.create"]),
    );
    namix::oauth::register(namix::DevProvider);
    crate::jobs::welcome_ping::register();
}
