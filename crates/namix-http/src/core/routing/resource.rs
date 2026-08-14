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

use crate::core::error::AppError;
use crate::core::request::Request;
use crate::core::response::{Respond, Response};

use super::{Route, Router};

/// Boxed resource action future. The lifetime permits an action to borrow its
/// controller while it awaits, and the result integrates with Namix's unified
/// application-error response boundary.
pub type ResourceFuture<'a> = Pin<Box<dyn Future<Output = Result<Response, AppError>> + Send + 'a>>;

/// Controller contract for the seven conventional CRUD endpoints.
///
/// Implement only the handlers a resource exposes; the default response is a
/// concise `405 Method Not Allowed` so APIs may deliberately omit e.g. HTML
/// `create`/`edit` pages.
pub trait ResourceController: Clone + Send + Sync + 'static {
    fn index(&self, _req: Request) -> ResourceFuture<'_> {
        Box::pin(async { Ok(method_not_allowed()) })
    }
    fn create(&self, _req: Request) -> ResourceFuture<'_> {
        Box::pin(async { Ok(method_not_allowed()) })
    }
    fn store(&self, _req: Request) -> ResourceFuture<'_> {
        Box::pin(async { Ok(method_not_allowed()) })
    }
    fn show(&self, _req: Request) -> ResourceFuture<'_> {
        Box::pin(async { Ok(method_not_allowed()) })
    }
    fn edit(&self, _req: Request) -> ResourceFuture<'_> {
        Box::pin(async { Ok(method_not_allowed()) })
    }
    fn update(&self, _req: Request) -> ResourceFuture<'_> {
        Box::pin(async { Ok(method_not_allowed()) })
    }
    fn destroy(&self, _req: Request) -> ResourceFuture<'_> {
        Box::pin(async { Ok(method_not_allowed()) })
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
    let update_patch = controller.clone();
    let update_put = controller.clone();
    let destroy = controller;

    Router::new()
        .merge(
            Route::get(&base, move |req: Request| {
                let controller = index.clone();
                async move { respond_resource(controller, req, ResourceController::index).await }
            })
            .name(format!("{resource}.index"))
            .register(),
        )
        .merge(
            Route::get(&create, move |req: Request| {
                let controller = create_controller.clone();
                async move { respond_resource(controller, req, ResourceController::create).await }
            })
            .name(format!("{resource}.create"))
            .register(),
        )
        .merge(
            Route::post(&base, move |req: Request| {
                let controller = store.clone();
                async move { respond_resource(controller, req, ResourceController::store).await }
            })
            .name(format!("{resource}.store"))
            .register(),
        )
        .merge(
            Route::get(&member, move |req: Request| {
                let controller = show.clone();
                async move { respond_resource(controller, req, ResourceController::show).await }
            })
            .name(format!("{resource}.show"))
            .register(),
        )
        .merge(
            Route::get(&edit, move |req: Request| {
                let controller = edit_controller.clone();
                async move { respond_resource(controller, req, ResourceController::edit).await }
            })
            .name(format!("{resource}.edit"))
            .register(),
        )
        .merge(
            Route::patch(&member, move |req: Request| {
                let controller = update_patch.clone();
                async move { respond_resource(controller, req, ResourceController::update).await }
            })
            .name(format!("{resource}.update"))
            .register(),
        )
        .merge(
            Route::put(&member, move |req: Request| {
                let controller = update_put.clone();
                async move { respond_resource(controller, req, ResourceController::update).await }
            })
            .name(format!("{resource}.update"))
            .register(),
        )
        .merge(
            Route::delete(&member, move |req: Request| {
                let controller = destroy.clone();
                async move { respond_resource(controller, req, ResourceController::destroy).await }
            })
            .name(format!("{resource}.destroy"))
            .register(),
        )
}

async fn respond_resource<C>(
    controller: C,
    req: Request,
    action: for<'a> fn(&'a C, Request) -> ResourceFuture<'a>,
) -> Response
where
    C: ResourceController,
{
    let response_request = req.clone();
    action(&controller, req).await.respond(&response_request)
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
        fn index(&self, _req: Request) -> ResourceFuture<'_> {
            Box::pin(async { Ok(crate::core::controller::text("index")) })
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
        let update = catalog.export().get("posts.update").cloned().unwrap();
        assert!(update.methods.iter().any(|method| method == "PATCH"));
        assert!(update.methods.iter().any(|method| method == "PUT"));
    }

    #[derive(Clone)]
    struct StatefulPosts {
        title: String,
    }

    impl StatefulPosts {
        async fn load_title(&self) -> Result<&str, AppError> {
            if self.title.is_empty() {
                Err(AppError::NotFound)
            } else {
                Ok(&self.title)
            }
        }
    }

    impl ResourceController for StatefulPosts {
        fn index(&self, _req: Request) -> ResourceFuture<'_> {
            Box::pin(async move {
                let title = self.load_title().await?;
                Ok(crate::core::controller::text(title))
            })
        }

        fn show(&self, _req: Request) -> ResourceFuture<'_> {
            Box::pin(async { Err(AppError::NotFound) })
        }
    }

    #[tokio::test]
    async fn resource_actions_may_borrow_self_and_use_question_mark() {
        let router = resource(
            "posts",
            StatefulPosts {
                title: "borrowed title".into(),
            },
        );
        let mut client = crate::core::test_client::TestClient::new(router);

        assert_eq!(client.get("/posts").await.text(), "borrowed title");
        assert_eq!(
            client.get("/posts/1").await.status,
            http::StatusCode::NOT_FOUND
        );
    }
}
