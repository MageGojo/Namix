use namix::prelude::*;

pub async fn index(_req: Request) -> Response {
    text("Dashboard")
}
