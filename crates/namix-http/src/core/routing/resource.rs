//! Laravel-style resource routing.
//!
//! ```ignore
//! #[derive(Clone)]
//! struct PostsController;
//! impl ResourceController for PostsController { /* … */ }
//! let router = resource("posts", PostsController);
//! // posts.index / posts.create / posts.store / posts.show / …
//! ```

use std::future::Future;
use std::pin::Pin;

use crate::core::request::Request;
use crate::core::response::Response;

use super::{Route, Router};

pub type ResourceFuture = Pin<Box<dyn Future<Output = Response> + Send>>;

/// Controller contract for the seven conventional CRUD endpoints.
///
/// Implement only the handlers a resource exposes; the default response is a
/// concise `405 Method Not Allowed` so APIs may deliberately omit e.g. HTML
/// `create`/`edit` pages.
pub trait ResourceController: Clone + Send + Sync + 'static {
    fn index(&self, _req: Request) -> ResourceFuture {
        Box::pin(async { method_not_allowed() })
    }
    fn create(&self, _req: Request) -> ResourceFuture {
        Box::pin(async { method_not_allowed() })
    }
    fn store(&self, _req: Request) -> ResourceFuture {
        Box::pin(async { method_not_allowed() })
    }
    fn show(&self, _req: Request) -> ResourceFuture {
        Box::pin(async { method_not_allowed() })
    }
    fn edit(&self, _req: Request) -> ResourceFuture {
        Box::pin(async { method_not_allowed() })
    }
    fn update(&self, _req: Request) -> ResourceFuture {
        Box::pin(async { method_not_allowed() })
    }
    fn destroy(&self, _req: Request) -> ResourceFuture {
        Box::pin(async { method_not_allowed() })
    }
}

/// Build the conventional CRUD route set. `name` is plural and becomes both
/// the URL segment and the route-name prefix (`posts.index`, `posts.show`, …).
pub fn resource<C>(name: &str, controller: C) -> Router
where
    C: ResourceController,
{
    let resource = normalize_resource(name);
    let base = format!("/{resource}");
    let create = format!("{base}/create");
    let member = format!("{base}/:id");
    let edit = format!("{member}/edit");

    let index = controller.clone();
    let create_controller = controller.clone();
    let store = controller.clone();
    let show = controller.clone();
    let edit_controller = controller.clone();
    let update = controller.clone();
    let destroy = controller;

    Router::new()
        .merge(
            Route::get(&base, move |req: Request| index.index(req))
                .name(format!("{resource}.index"))
                .register(),
        )
        .merge(
            Route::get(&create, move |req: Request| create_controller.create(req))
                .name(format!("{resource}.create"))
                .register(),
        )
        .merge(
            Route::post(&base, move |req: Request| store.store(req))
                .name(format!("{resource}.store"))
                .register(),
        )
        .merge(
            Route::get(&member, move |req: Request| show.show(req))
                .name(format!("{resource}.show"))
                .register(),
        )
        .merge(
            Route::get(&edit, move |req: Request| edit_controller.edit(req))
                .name(format!("{resource}.edit"))
                .register(),
        )
        .merge(
            Route::patch(&member, move |req: Request| update.update(req))
                .name(format!("{resource}.update"))
                .register(),
        )
        .merge(
            Route::delete(&member, move |req: Request| destroy.destroy(req))
                .name(format!("{resource}.destroy"))
                .register(),
        )
}

/// Alias for call sites that prefer `resources("posts", controller)`.
pub fn resources<C>(name: &str, controller: C) -> Router
where
    C: ResourceController,
{
    resource(name, controller)
}

fn normalize_resource(name: &str) -> String {
    name.trim_matches('/').trim().to_string()
}

fn method_not_allowed() -> Response {
    Response::new(
        http::StatusCode::METHOD_NOT_ALLOWED,
        crate::core::content_type::ContentType::Text,
        "resource action is not implemented",
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Clone)]
    struct Posts;

    impl ResourceController for Posts {
        fn index(&self, _req: Request) -> ResourceFuture {
            Box::pin(async { crate::core::controller::text("index") })
        }
    }

    #[test]
    fn creates_all_seven_named_routes() {
        let catalog = resource("posts", Posts).catalog();
        for name in [
            "posts.index",
            "posts.create",
            "posts.store",
            "posts.show",
            "posts.edit",
            "posts.update",
            "posts.destroy",
        ] {
            assert!(catalog.path(name).is_some(), "missing {name}");
        }
    }
}
