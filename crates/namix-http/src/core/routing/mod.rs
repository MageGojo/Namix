mod catalog;
pub(crate) mod path;
mod resource;
mod route;
mod router;
mod ts_routes;

pub use catalog::{IntoRouteName, NamedRoute, RouteCatalog, RouteExport};
pub use route::Route;
pub use router::Router;

use crate::core::middleware::BoxFuture;
use crate::core::request::Request;
use crate::core::response::Response;

use std::sync::Arc;

pub(crate) type HandlerFn = Arc<dyn Fn(Request) -> BoxFuture<Response> + Send + Sync + 'static>;

/// 供 `routes!` 宏拼接组前缀与子路径。
pub fn path_join(prefix: &str, child: &str) -> String {
    path::PathPattern::join(prefix, child)
}

pub use resource::{ResourceController, ResourceFuture, resource, resources};
