use namix::prelude::*;

async fn ok(_req: Request) -> Response {
    text("ok")
}

async fn socket(_socket: WsSocket) {}

#[test]
fn routes_macro_builds_http_patch_and_websocket_catalog() {
    let router = routes! {
        "/api" => {
            GET "/users" => ok, name: "users.index",
            PATCH "/users/:id" => ok, name: "users.update",
            WS "/events" => socket, name: "users.events",
        },
    };

    let exported = router.catalog().export();
    assert_eq!(exported["users.index"].uri, "/api/users");
    assert_eq!(exported["users.index"].methods, ["GET"]);
    assert_eq!(exported["users.update"].uri, "/api/users/:id");
    assert_eq!(exported["users.update"].methods, ["PATCH"]);
    assert_eq!(exported["users.events"].uri, "/api/events");
    assert_eq!(exported["users.events"].methods, ["WS"]);
}
