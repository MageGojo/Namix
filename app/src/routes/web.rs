use namix::prelude::*;

use crate::controllers::{auth, chat, demo, home, mailbox, me, posts, profile, realtime};
use crate::middleware::auth::{require_guest, require_login, require_vip};

/// 路由表。
///
/// 登录/注册/登出等写操作由 `#[server]` 自动挂到 `POST /api/a`；
/// 这里只保留页面、经典表单、SSE 与 WebSocket 路由。
pub fn routes() -> Router {
    routes! {
        "/" => {
            GET "/" => home::index, name: "home",
            GET "/login" => auth::login, name: "login",
                middleware = [require_guest],
            GET "/register" => auth::register, name: "register",
                middleware = [require_guest],
            POST "/logout" => auth::logout_page, name: "logout",
            GET "/demo" => demo::ssr, name: "demo",
            GET "/island" => demo::island, name: "island",
            GET "/sse/ticks" => realtime::ticks, name: "sse.ticks",
            WS "/ws/echo" => realtime::echo, name: "ws.echo",
            WS "/ws/chat" => chat::socket, name: "ws.chat",
            POST "/webhooks/mail/inbound" => mailbox::inbound_webhook, name: "webhooks.mail.inbound",
            GET "/vip" => home::vip_lounge, name: "vip",
                middleware = [require_login, require_vip],
        },
        "/" => {
            GET "/me" => me::show, name: "me",
            POST "/me" => me::save, name: "me.submit",
            GET "/posts" => posts::index, name: "posts",
            POST "/posts" => posts::create, name: "posts.submit",
            POST "/posts/:id" => posts::update, name: "posts.update",
            POST "/posts/:id/delete" => posts::destroy, name: "posts.destroy",
            GET "/profile/:id" => profile::show, name: "profile",
            GET "/chat" => chat::page, name: "chat",
            GET "/mailbox" => mailbox::page, name: "mailbox",
            middleware: [require_login],
        },
    }
}
