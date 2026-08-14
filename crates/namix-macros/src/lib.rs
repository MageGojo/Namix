//! Namix 路由宏：`routes!` / `#[route]` / `#[server]` / `ViewProps` / `FormField`。

use proc_macro::TokenStream;
use quote::quote;
use syn::ext::IdentExt;
use syn::parse::{Parse, ParseStream};
use syn::punctuated::Punctuated;
use syn::{
    Data, DeriveInput, Expr, Fields, FnArg, Ident, ItemFn, LitStr, Pat, Token, Type,
    parse_macro_input,
};

/// `#[server]` / `#[server(name = "register", seal = ["password"])]`
///
/// Leptos 风格 Server Function：
/// - 客户端 `callRust` → WASM 单次 `POST /api/a`（包络含动作 token；可整包密封）
/// - 生成 `views/generated/actions/{name}.ts` 供 TSX 调用
///
/// ```ignore
/// #[server(name = "register", seal = ["password", "password_confirmation"])]
/// pub async fn register(input: RegisterInput) -> Result<ActionOk<AuthOk>, String> {
///     ...
/// }
/// ```
#[proc_macro_attribute]
pub fn server(attr: TokenStream, item: TokenStream) -> TokenStream {
    let args = parse_macro_input!(attr as ServerAttr);
    let func = parse_macro_input!(item as ItemFn);

    if func.sig.asyncness.is_none() {
        return syn::Error::new_spanned(func.sig.fn_token, "#[server] requires async fn")
            .to_compile_error()
            .into();
    }

    let fn_name = &func.sig.ident;
    let fn_name_str = fn_name.unraw().to_string();
    let action_name = args
        .name
        .as_ref()
        .map(|s| s.value())
        .unwrap_or_else(|| fn_name_str.clone());
    if let Err(message) = validate_action_name(&action_name) {
        let span = args
            .name
            .as_ref()
            .map(LitStr::span)
            .unwrap_or_else(|| fn_name.span());
        return syn::Error::new(span, message).to_compile_error().into();
    }

    let vis = &func.vis;
    let attrs = &func.attrs;
    let sig = &func.sig;
    let body = &func.block;

    let handler_ident = Ident::new(
        &format!("__namix_server_handler_{fn_name_str}"),
        fn_name.span(),
    );
    let call_ident = Ident::new(
        &format!("__namix_server_call_{fn_name_str}"),
        fn_name.span(),
    );

    let seal_lits: Vec<_> = args.seal.iter().collect();
    let seal_tokens = quote! { &[#(#seal_lits),*] };

    // 入参：() / (Request) / (T) / (Request, T)
    let inputs: Vec<_> = sig.inputs.iter().collect();
    let call_tokens = match server_call_tokens(fn_name, inputs.as_slice()) {
        Ok(t) => t,
        Err(e) => return e.to_compile_error().into(),
    };

    // `(Request)` may consume a browser-supplied form/JSON body even though the
    // Rust function receives no separate DTO. Keep that input optional in TS;
    // sealing controls transport only and must not change the function arity.
    let input_mode = server_action_input_mode(inputs.as_slice());
    let action_tok = fnv_action_token(&action_name);
    write_server_action_ts(&action_name, &action_tok, input_mode);

    quote! {
        #(#attrs)*
        #vis #sig #body

        #[allow(non_snake_case)]
        async fn #handler_ident(req: ::namix::Request) -> ::namix::Response {
            let __nav = ::namix::server_fn::wants_html_navigation(&req);
            let __out = #call_tokens;
            let __resp =
                ::namix::server_fn::IntoActionResponse::into_action_response(__out);
            ::namix::server_fn::finalize_action(__nav, __resp).await
        }

        #[allow(non_snake_case)]
        fn #call_ident(
            req: ::namix::Request,
        ) -> ::std::pin::Pin<
            ::std::boxed::Box<
                dyn ::std::future::Future<Output = ::namix::Response> + ::std::marker::Send,
            >,
        > {
            ::std::boxed::Box::pin(#handler_ident(req))
        }

        ::namix::server_fn::inventory::submit! {
            ::namix::server_fn::ServerFn {
                name: #action_name,
                token: #action_tok,
                seal: #seal_tokens,
                call: #call_ident,
            }
        }
    }
    .into()
}

/// 与 `server_fn::action_token` 同算法（宏内编译期算 token）。
fn fnv_action_token(name: &str) -> String {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in name.as_bytes() {
        h ^= u64::from(*b);
        h = h.wrapping_mul(0x100000001b3);
    }
    format!("{h:016x}")[..8].to_string()
}

fn is_request_type(ty: &Type) -> bool {
    matches!(type_path_name(ty).as_deref(), Some("Request"))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ServerActionInputMode {
    None,
    Optional,
    Required,
}

fn server_action_input_mode(inputs: &[&FnArg]) -> ServerActionInputMode {
    match inputs {
        [] => ServerActionInputMode::None,
        [FnArg::Typed(pt)] if is_request_type(&pt.ty) => ServerActionInputMode::Optional,
        _ => ServerActionInputMode::Required,
    }
}

fn validate_action_name(name: &str) -> Result<(), String> {
    let mut chars = name.chars();
    let Some(first) = chars.next() else {
        return Err("#[server] action name must not be empty".into());
    };
    if !(first.is_ascii_alphabetic() || first == '_')
        || !chars.all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
    {
        return Err(format!(
            "invalid #[server] action name `{name}`: expected an ASCII Rust/TypeScript identifier (`[A-Za-z_][A-Za-z0-9_]*`)"
        ));
    }
    if syn::parse_str::<Ident>(name).is_err() || is_reserved_action_identifier(name) {
        return Err(format!(
            "invalid #[server] action name `{name}`: language keywords are not valid generated function names"
        ));
    }
    if is_reserved_filename(name) {
        return Err(format!(
            "invalid #[server] action name `{name}`: the generated filename is reserved"
        ));
    }
    Ok(())
}

fn is_reserved_action_identifier(name: &str) -> bool {
    matches!(
        name,
        // Rust keywords and reserved words.
        "as" | "break" | "const" | "continue" | "crate" | "else" | "enum"
            | "extern" | "false" | "fn" | "for" | "if" | "impl" | "in" | "let"
            | "loop" | "match" | "mod" | "move" | "mut" | "pub" | "ref" | "return"
            | "self" | "Self" | "static" | "struct" | "super" | "trait" | "true"
            | "type" | "unsafe" | "use" | "where" | "while" | "async" | "await"
            | "dyn" | "abstract" | "become" | "box" | "do" | "final" | "macro"
            | "override" | "priv" | "typeof" | "unsized" | "virtual" | "yield" | "try"
            | "union" | "gen"
            // JavaScript/TypeScript keywords not already listed above.
            | "case" | "catch" | "class" | "debugger" | "default" | "delete" | "export"
            | "extends" | "finally" | "function" | "import" | "instanceof" | "new"
            | "null" | "switch" | "this" | "throw" | "var" | "void" | "with"
            | "implements" | "interface" | "package" | "private" | "protected"
            | "public" | "arguments" | "eval"
    )
}

fn is_reserved_filename(name: &str) -> bool {
    let upper = name.to_ascii_uppercase();
    matches!(upper.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || (upper.len() == 4
            && (upper.starts_with("COM") || upper.starts_with("LPT"))
            && matches!(upper.as_bytes()[3], b'1'..=b'9'))
}

fn server_call_tokens(
    fn_name: &Ident,
    inputs: &[&FnArg],
) -> Result<proc_macro2::TokenStream, syn::Error> {
    match inputs {
        [] => Ok(quote! { #fn_name().await }),
        [FnArg::Typed(pt)] if is_request_type(&pt.ty) => {
            let pat = require_ident_pat(&pt.pat)?;
            Ok(quote! {
                {
                    let #pat = req;
                    #fn_name(#pat).await
                }
            })
        }
        [FnArg::Typed(pt)] => {
            let pat = require_ident_pat(&pt.pat)?;
            let ty = &pt.ty;
            Ok(quote! {
                {
                    let #pat = match ::namix::server_fn::parse_json_body::<#ty>(&req) {
                        Ok(v) => v,
                        Err(resp) => return resp,
                    };
                    #fn_name(#pat).await
                }
            })
        }
        [FnArg::Typed(req_pt), FnArg::Typed(body_pt)] if is_request_type(&req_pt.ty) => {
            let req_pat = require_ident_pat(&req_pt.pat)?;
            let body_pat = require_ident_pat(&body_pt.pat)?;
            let body_ty = &body_pt.ty;
            Ok(quote! {
                {
                    let #req_pat = req;
                    let #body_pat =
                        match ::namix::server_fn::parse_json_body::<#body_ty>(&#req_pat) {
                            Ok(v) => v,
                            Err(resp) => return resp,
                        };
                    #fn_name(#req_pat, #body_pat).await
                }
            })
        }
        _ => Err(syn::Error::new_spanned(
            inputs
                .first()
                .map(|a| quote! { #a })
                .unwrap_or_else(|| quote! {}),
            "#[server] supports (), (Request), (T), or (Request, T)",
        )),
    }
}

fn require_ident_pat(pat: &Pat) -> Result<&Pat, syn::Error> {
    if matches!(pat, Pat::Ident(_)) {
        Ok(pat)
    } else {
        Err(syn::Error::new_spanned(
            pat,
            "#[server] argument must be a simple ident",
        ))
    }
}

struct ServerAttr {
    name: Option<LitStr>,
    seal: Vec<LitStr>,
}

impl Parse for ServerAttr {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let mut name = None;
        let mut seal = Vec::new();
        if input.is_empty() {
            return Ok(Self { name, seal });
        }
        let args = Punctuated::<ServerAttrArg, Token![,]>::parse_terminated(input)?;
        for arg in args {
            match arg {
                ServerAttrArg::Name(s) => name = Some(s),
                ServerAttrArg::Seal(list) => seal = list,
            }
        }
        Ok(Self { name, seal })
    }
}

enum ServerAttrArg {
    Name(LitStr),
    Seal(Vec<LitStr>),
}

impl Parse for ServerAttrArg {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let key: Ident = input.parse()?;
        input.parse::<Token![=]>()?;
        if key == "name" {
            Ok(Self::Name(input.parse()?))
        } else if key == "seal" {
            let content;
            syn::bracketed!(content in input);
            let list = Punctuated::<LitStr, Token![,]>::parse_terminated(&content)?;
            Ok(Self::Seal(list.into_iter().collect()))
        } else {
            Err(syn::Error::new(
                key.span(),
                "expected `name = \"...\"` or `seal = [\"...\"]`",
            ))
        }
    }
}

fn write_server_action_ts(action_name: &str, action_tok: &str, input_mode: ServerActionInputMode) {
    let body = render_server_action_ts(action_name, action_tok, input_mode);

    let Ok(manifest) = std::env::var("CARGO_MANIFEST_DIR") else {
        return;
    };
    let dir = std::path::PathBuf::from(manifest).join("src/views/generated/actions");
    let _ = std::fs::create_dir_all(&dir);
    let path = dir.join(format!("{action_name}.ts"));
    if std::fs::read_to_string(&path).ok().as_deref() != Some(body.as_str()) {
        let _ = std::fs::write(&path, body);
    }
    rewrite_actions_barrel(&dir);
}

fn render_server_action_ts(
    action_name: &str,
    action_tok: &str,
    input_mode: ServerActionInputMode,
) -> String {
    let fn_ts = match input_mode {
        ServerActionInputMode::Required => format!(
            "/** action `{action_name}` → token `{action_tok}`（路径已混淆） */\n\
             export function {action_name}(input: Record<string, unknown>): Promise<Record<string, unknown>> {{\n\
             \treturn callRust('{action_tok}', input)\n\
             }}\n"
        ),
        ServerActionInputMode::Optional => format!(
            "/** action `{action_name}` → token `{action_tok}`（路径已混淆） */\n\
             export function {action_name}(input?: Record<string, unknown>): Promise<Record<string, unknown>> {{\n\
             \treturn callRust('{action_tok}', input)\n\
             }}\n"
        ),
        ServerActionInputMode::None => format!(
            "/** action `{action_name}` → token `{action_tok}` */\n\
             export function {action_name}(): Promise<Record<string, unknown>> {{\n\
             \treturn callRust('{action_tok}')\n\
             }}\n"
        ),
    };

    format!(
        "/* @generated by #[server(\"{action_name}\")] — DO NOT EDIT */\n\
         import {{ callRust }} from '../callRust'\n\n\
         {fn_ts}"
    )
}

fn rewrite_actions_barrel(actions_dir: &std::path::Path) {
    let mut names: Vec<String> = std::fs::read_dir(actions_dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let path = e.path();
            if path.extension().and_then(|x| x.to_str()) != Some("ts") {
                return None;
            }
            let stem = path.file_stem()?.to_str()?.to_string();
            if stem == "index" {
                return None;
            }
            Some(stem)
        })
        .collect();
    names.sort();
    let mut body = String::from(
        "/* @generated by #[server] — DO NOT EDIT */\n\
         /* import { server } from '../generated' → server.login(...) */\n",
    );
    for name in &names {
        body.push_str(&format!("export {{ {name} }} from './{name}'\n"));
    }
    let path = actions_dir.join("index.ts");
    if std::fs::read_to_string(&path).ok().as_deref() != Some(body.as_str()) {
        let _ = std::fs::write(path, body);
    }
}

fn type_path_name(ty: &Type) -> Option<String> {
    let Type::Path(p) = ty else {
        return None;
    };
    Some(p.path.segments.last()?.ident.to_string())
}

/// `#[route(GET, "/users/:id", name = "user.show", middleware = [auth])]`
///
/// 展开为模块：`get_user::handler` + `get_user::router()`。
#[proc_macro_attribute]
pub fn route(attr: TokenStream, item: TokenStream) -> TokenStream {
    let args = parse_macro_input!(attr as RouteAttr);
    let func = parse_macro_input!(item as ItemFn);

    let vis = &func.vis;
    let fn_name = &func.sig.ident;
    let fn_async = &func.sig.asyncness;
    let inputs = &func.sig.inputs;
    let output = &func.sig.output;
    let body = &func.block;
    let attrs = &func.attrs;

    let method_ident = args.method.to_string().to_uppercase();
    let method_expr = match method_ident.as_str() {
        "GET" => quote!(::namix::http::Method::GET),
        "POST" => quote!(::namix::http::Method::POST),
        "PUT" => quote!(::namix::http::Method::PUT),
        "DELETE" => quote!(::namix::http::Method::DELETE),
        "PATCH" => quote!(::namix::http::Method::PATCH),
        other => {
            return syn::Error::new_spanned(args.method, format!("unsupported method: {other}"))
                .to_compile_error()
                .into();
        }
    };

    let path = args.path;
    let name_tokens = match args.name {
        Some(name) => quote!(.name(#name)),
        None => quote!(),
    };
    let mw_tokens = args.middlewares.iter().map(|mw| quote!(.middleware(#mw)));

    quote! {
        #vis mod #fn_name {
            use ::namix::prelude::*;
            #[allow(unused_imports)]
            use super::*;

            #(#attrs)*
            pub #fn_async fn handler(#inputs) #output #body

            pub fn router() -> ::namix::Router {
                ::namix::Route::new(#method_expr, #path, handler)
                    #name_tokens
                    #(#mw_tokens)*
                    .register()
            }
        }
    }
    .into()
}

struct RouteAttr {
    method: Ident,
    path: LitStr,
    name: Option<LitStr>,
    middlewares: Vec<Expr>,
}

impl Parse for RouteAttr {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let method: Ident = input.parse()?;
        input.parse::<Token![,]>()?;
        let path: LitStr = input.parse()?;
        let mut name = None;
        let mut middlewares = Vec::new();

        while input.parse::<Token![,]>().is_ok() {
            let key: Ident = input.parse()?;
            input.parse::<Token![=]>()?;
            if key == "name" {
                name = Some(input.parse()?);
            } else if key == "middleware" {
                let content;
                syn::bracketed!(content in input);
                middlewares = Punctuated::<Expr, Token![,]>::parse_terminated(&content)?
                    .into_iter()
                    .collect();
            } else {
                return Err(syn::Error::new(
                    key.span(),
                    "expected `name = \"...\"` or `middleware = [...]`",
                ));
            }
        }

        Ok(Self {
            method,
            path,
            name,
            middlewares,
        })
    }
}

/// 分组路由宏。
///
/// - 组中间件：`middleware: [a, b]`（独立一行，`:`）
/// - 单路由中间件：`middleware = [a]`（HTTP 与 WS 均支持）
///
/// ```ignore
/// routes! {
///     "/api" => {
///         GET "/greeting" => || "Hello World", name: "greeting",
///         GET "/me" => user::me, name: "user.me", middleware = [auth],
///         WS "/events" => user::events, name: "user.events", middleware = [auth],
///         middleware: [logger],
///     },
/// }
/// ```
#[proc_macro]
pub fn routes(input: TokenStream) -> TokenStream {
    let file = parse_macro_input!(input as RoutesFile);
    let groups = file.groups.iter().map(|group| {
        let prefix = &group.prefix;
        let routes = group.routes.iter().map(|r| {
            let method = r.method.to_string().to_uppercase();
            let path = &r.path;
            let handler = &r.handler;
            let name_tokens = match &r.name {
                Some(name) => quote!(.name(#name)), // LitStr 或 route::user::login
                None => quote!(),
            };
            let mw_tokens = r.middlewares.iter().map(|mw| quote!(.middleware(#mw)));

            if method == "WS" {
                return quote! {
                    {
                        let full = ::namix::routing_path_join(#prefix, #path);
                        let route = ::namix::Route::ws(&full, #handler)
                            #name_tokens
                            #(#mw_tokens)*;
                        __group_router = __group_router.merge(route.register());
                    }
                };
            }

            let method_expr = match method.as_str() {
                "GET" => quote!(::namix::http::Method::GET),
                "POST" => quote!(::namix::http::Method::POST),
                "PUT" => quote!(::namix::http::Method::PUT),
                "DELETE" => quote!(::namix::http::Method::DELETE),
                "PATCH" => quote!(::namix::http::Method::PATCH),
                other => {
                    return syn::Error::new_spanned(
                        &r.method,
                        format!("unsupported method: {other}"),
                    )
                    .to_compile_error();
                }
            };
            quote! {
                {
                    let full = ::namix::routing_path_join(#prefix, #path);
                    let route = ::namix::Route::new(#method_expr, &full, #handler)
                        #name_tokens
                        #(#mw_tokens)*;
                    __group_router = __group_router.merge(route.register());
                }
            }
        });

        // Router::middleware adds an outer layer, so apply the list in reverse
        // to preserve the source order at request time.
        let mws = group.middlewares.iter().rev().map(|mw| {
            quote! {
                __group_router = __group_router.middleware(#mw);
            }
        });

        quote! {
            {
                let mut __group_router = ::namix::Router::new();
                #(#routes)*
                #(#mws)*
                __router = __router.merge(__group_router);
            }
        }
    });

    quote! {
        {
            let mut __router = ::namix::Router::new();
            #(#groups)*
            __router
        }
    }
    .into()
}

struct RoutesFile {
    groups: Vec<RouteGroup>,
}

impl Parse for RoutesFile {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let mut groups = Vec::new();
        while !input.is_empty() {
            groups.push(input.parse()?);
            let _ = input.parse::<Token![,]>();
        }
        Ok(Self { groups })
    }
}

struct RouteGroup {
    prefix: LitStr,
    routes: Vec<GroupRoute>,
    middlewares: Vec<Expr>,
}

impl Parse for RouteGroup {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let prefix: LitStr = input.parse()?;
        input.parse::<Token![=>]>()?;
        let content;
        syn::braced!(content in input);

        let mut routes = Vec::new();
        let mut middlewares = Vec::new();

        while !content.is_empty() {
            if content.peek(Ident) {
                let ident: Ident = content.fork().parse()?;
                if ident == "middleware" {
                    content.parse::<Ident>()?;
                    content.parse::<Token![:]>()?;
                    let mw_content;
                    syn::bracketed!(mw_content in content);
                    middlewares = Punctuated::<Expr, Token![,]>::parse_terminated(&mw_content)?
                        .into_iter()
                        .collect();
                    let _ = content.parse::<Token![,]>();
                    continue;
                }
            }
            routes.push(content.parse()?);
            let _ = content.parse::<Token![,]>();
        }

        Ok(Self {
            prefix,
            routes,
            middlewares,
        })
    }
}

struct GroupRoute {
    method: Ident,
    path: LitStr,
    handler: Expr,
    name: Option<Expr>,
    middlewares: Vec<Expr>,
}

impl Parse for GroupRoute {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let method: Ident = input.parse()?;
        let path: LitStr = input.parse()?;
        input.parse::<Token![=>]>()?;
        let handler: Expr = input.parse()?;

        let mut name = None;
        let mut middlewares = Vec::new();

        while input.peek(Token![,]) {
            let probe = input.fork();
            probe.parse::<Token![,]>()?;
            if !probe.peek(Ident) {
                break;
            }
            let key: Ident = probe.parse()?;
            if key == "name" {
                input.parse::<Token![,]>()?;
                input.parse::<Ident>()?;
                input.parse::<Token![:]>()?;
                name = Some(input.parse::<Expr>()?);
                continue;
            }
            // 单路由：`middleware = [...]`；组级：`middleware: [...]` 不在这里消费
            if key == "middleware" && probe.peek(Token![=]) {
                input.parse::<Token![,]>()?;
                input.parse::<Ident>()?;
                input.parse::<Token![=]>()?;
                let mw_content;
                syn::bracketed!(mw_content in input);
                middlewares = Punctuated::<Expr, Token![,]>::parse_terminated(&mw_content)?
                    .into_iter()
                    .collect();
                continue;
            }
            break;
        }

        Ok(Self {
            method,
            path,
            handler,
            name,
            middlewares,
        })
    }
}

/// `#[derive(FormField)]` — 验证字段 enum，带补全。
///
/// ```ignore
/// #[derive(FormField)]
/// enum LoginForm {
///     #[field = "email"]
///     Email,
///     Password, // 默认小写变体名 → "password"
/// }
/// ```
#[proc_macro_derive(FormField, attributes(field))]
pub fn derive_form_field(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    let enum_ident = &input.ident;
    let Data::Enum(data) = &input.data else {
        return syn::Error::new_spanned(&input.ident, "FormField only on enums")
            .to_compile_error()
            .into();
    };

    let mut arms = Vec::new();
    let mut pairs = Vec::new();
    for v in &data.variants {
        let variant = &v.ident;
        let mut value = None;
        for attr in &v.attrs {
            if attr.path().is_ident("field") {
                if let Ok(list) = attr.meta.require_list() {
                    if let Ok(lit) = list.parse_args::<LitStr>() {
                        value = Some(lit.value());
                    }
                } else if let Ok(name_val) = attr.meta.require_name_value()
                    && let syn::Expr::Lit(syn::ExprLit {
                        lit: syn::Lit::Str(s),
                        ..
                    }) = &name_val.value
                {
                    value = Some(s.value());
                }
            }
        }
        let value = value.unwrap_or_else(|| to_snake_or_lower(&variant.to_string()));
        let variant_str = variant.to_string();
        arms.push(quote! { Self::#variant => #value, });
        pairs.push(quote! { (#variant_str, #value) });
    }

    quote! {
        impl ::namix::validate::Field for #enum_ident {
            fn name(self) -> &'static str {
                match self {
                    #(#arms)*
                }
            }
        }

        impl #enum_ident {
            /// `(Variant, field_name)` — 供 TS 契约 / 文档。
            pub const FIELDS: &'static [(&'static str, &'static str)] = &[#(#pairs),*];
        }
    }
    .into()
}

/// 页面 props：一次定义，控制器 `Home { .. }.render(&req)`，TSX 用生成的类型。
///
/// ```ignore
/// #[derive(Serialize, ViewProps)]
/// #[view("home")]                         // spa（默认）
/// #[view("demo", mode = "ssr")]           // 纯 SSR：HTML + CSS
/// #[view("widget", mode = "island")]      // SSR + 客户端可交互（后续拆岛）
/// #[serde(rename_all = "camelCase")]
/// pub struct Home {
///     pub title: String,
///     pub users_count: u64,
/// }
/// ```
#[proc_macro_derive(ViewProps, attributes(view, view_title))]
pub fn derive_view_props(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    let struct_ident = &input.ident;
    let Data::Struct(data) = &input.data else {
        return syn::Error::new_spanned(&input.ident, "ViewProps only on structs")
            .to_compile_error()
            .into();
    };
    let Fields::Named(fields) = &data.fields else {
        return syn::Error::new_spanned(&input.ident, "ViewProps needs named fields")
            .to_compile_error()
            .into();
    };

    let mut component = to_snake_or_lower(&struct_ident.to_string());
    let mut static_title: Option<String> = None;
    let mut render_mode = String::from("spa");
    let rename_all = serde_rename_all(&input.attrs);
    for attr in &input.attrs {
        if !attr.path().is_ident("view") {
            continue;
        }
        if let Ok(list) = attr.meta.require_list() {
            // #[view("login")] | #[view("login", mode = "ssr")] | #[view(name = "..", mode = "..")]
            if let Ok(args) = list.parse_args_with(parse_view_attr_args) {
                if let Some(name) = args.name {
                    component = name;
                }
                if let Some(title) = args.title {
                    static_title = Some(title);
                }
                if let Some(mode) = args.mode {
                    render_mode = mode;
                }
            }
        } else if let Ok(nv) = attr.meta.require_name_value()
            && let syn::Expr::Lit(syn::ExprLit {
                lit: syn::Lit::Str(s),
                ..
            }) = &nv.value
        {
            component = s.value();
        }
    }

    let mode_tokens = match render_mode.to_ascii_lowercase().as_str() {
        "ssr" | "server" | "static" | "ssg" | "html" => {
            quote! { ::namix::pages::RenderMode::Ssr }
        }
        "island" | "hydrate" => {
            quote! { ::namix::pages::RenderMode::Island }
        }
        _ => quote! { ::namix::pages::RenderMode::Spa },
    };

    let mut title_field: Option<syn::Ident> = None;
    let mut ts_fields = Vec::new();
    let mut ts_imports: Vec<String> = Vec::new();
    for field in &fields.named {
        let name = field.ident.as_ref().unwrap();
        for attr in &field.attrs {
            if attr.path().is_ident("view_title") {
                title_field = Some(name.clone());
            }
        }
        let json_name = serde_field_name(field, name, rename_all.as_deref());
        let mapped = rust_type_to_ts(&field.ty);
        for dep in &mapped.imports {
            if !ts_imports.iter().any(|x| x == dep) {
                ts_imports.push(dep.clone());
            }
        }
        ts_fields.push(format!("  {json_name}: {};", mapped.ts));
    }

    let title_tokens = if let Some(ref f) = title_field {
        quote! { Some(self.#f.as_str()) }
    } else if let Some(ref t) = static_title {
        quote! { Some(#t) }
    } else {
        quote! { None }
    };

    let ts_name = format!("{}Props", struct_ident);
    let import_lines: String = ts_imports
        .iter()
        .map(|t| format!("import type {{ {t} }} from './{t}';\n"))
        .collect();
    let ts_body = format!(
        "/* @generated by #[derive(ViewProps)] — DO NOT EDIT */\n\
         {imports}\
         export const {const_name}_VIEW = \"{component}\" as const;\n\
         export type {ts_name} = {{\n{fields}\n}};\n",
        imports = import_lines,
        const_name = to_shouty(&struct_ident.to_string()),
        component = component,
        ts_name = ts_name,
        fields = ts_fields.join("\n"),
    );
    let file_stem = component.replace('/', "_");
    write_generated_ts(&file_stem, &ts_body);

    quote! {
        impl ::namix::pages::ViewPage for #struct_ident
        where
            Self: ::serde::Serialize,
        {
            const COMPONENT: &'static str = #component;
            const RENDER_MODE: ::namix::pages::RenderMode = #mode_tokens;

            fn document_title(&self) -> Option<&str> {
                #title_tokens
            }
        }

        impl #struct_ident
        where
            Self: ::serde::Serialize,
        {
            /// 渲染到对应 TSX（`app/src/views/{component}.tsx`）。
            pub fn render(self, req: &::namix::Request) -> ::namix::Response {
                ::namix::pages::ViewPage::render_page(self, req)
            }
        }
    }
    .into()
}

struct ViewAttrArgs {
    name: Option<String>,
    title: Option<String>,
    mode: Option<String>,
}

fn parse_view_attr_args(input: ParseStream<'_>) -> syn::Result<ViewAttrArgs> {
    let mut args = ViewAttrArgs {
        name: None,
        title: None,
        mode: None,
    };

    if input.peek(LitStr) {
        let lit: LitStr = input.parse()?;
        args.name = Some(lit.value());
        if input.is_empty() {
            return Ok(args);
        }
        input.parse::<Token![,]>()?;
    }

    while !input.is_empty() {
        let key: Ident = input.parse()?;
        input.parse::<Token![=]>()?;
        let val: LitStr = input.parse()?;
        match key.to_string().as_str() {
            "name" => args.name = Some(val.value()),
            "title" => args.title = Some(val.value()),
            "mode" => args.mode = Some(val.value()),
            other => {
                return Err(syn::Error::new(
                    key.span(),
                    format!("unknown view attr `{other}`; expected name/title/mode"),
                ));
            }
        }
        if input.is_empty() {
            break;
        }
        input.parse::<Token![,]>()?;
    }
    Ok(args)
}

/// `#[derive(NamedRoute)]` — 路由名 enum，带补全。
///
/// 更推荐 [`route_names!`]：`route::user::login`。
#[proc_macro_derive(NamedRoute, attributes(route))]
pub fn derive_named_route(input: TokenStream) -> TokenStream {
    derive_str_enum(
        input,
        "route",
        "NamedRoute",
        "route_name",
        "::namix::routing::NamedRoute",
    )
}

/// 生成类型化路由名模块：枚举 `AppRoute` + 常量别名 `route::main::login`。
///
/// ```ignore
/// namix::route_names! {
///     main {
///         home => "/",
///         login => "/login",
///         me_submit = "me.submit" => "/me",
///         profile = "profile" => "/profile/:id",
///     }
/// }
///
/// req.redirect_guest_to(AppRoute::Login);
/// req.redirect_guest_to(route::main::login);
/// AppRoute::Profile.to(&[("id", "1")]) // → /profile/1
/// ```
#[proc_macro]
pub fn route_names(input: TokenStream) -> TokenStream {
    let file = parse_macro_input!(input as RouteNamesFile);
    let reexport = if file.apps.iter().any(|app| app.name == "main") {
        Some(quote! { pub use main::AppRoute; })
    } else if file.apps.len() == 1 {
        let name = &file.apps[0].name;
        Some(quote! { pub use #name::AppRoute; })
    } else {
        None
    };
    let apps = file.apps.iter().map(|app| {
        let app_ident = &app.name;
        let app_str = app.name.to_string();
        let variants: Vec<_> = app
            .leaves
            .iter()
            .map(|leaf| {
                Ident::new(
                    &enum_variant_ident(&leaf.ident.to_string()),
                    leaf.ident.span(),
                )
            })
            .collect();
        let name_arms = app
            .leaves
            .iter()
            .zip(variants.iter())
            .map(|(leaf, variant)| {
                let route_str = leaf
                    .override_name
                    .as_ref()
                    .map(|s| s.value())
                    .unwrap_or_else(|| {
                        format!("{app_str}.{}", leaf.ident.to_string().replace('_', "."))
                    });
                quote! { Self::#variant => #route_str }
            });
        let uri_arms = app
            .leaves
            .iter()
            .zip(variants.iter())
            .map(|(leaf, variant)| match &leaf.uri {
                Some(uri) => {
                    let value = uri.value();
                    quote! { Self::#variant => Some(#value) }
                }
                None => quote! { Self::#variant => None },
            });
        let consts = app
            .leaves
            .iter()
            .zip(variants.iter())
            .map(|(leaf, variant)| {
                let const_ident = &leaf.ident;
                quote! {
                    #[allow(non_upper_case_globals)]
                    pub const #const_ident: AppRoute = AppRoute::#variant;
                }
            });
        quote! {
            pub mod #app_ident {
                #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
                pub enum AppRoute {
                    #(#variants,)*
                }

                impl ::namix::routing::NamedRoute for AppRoute {
                    #[inline]
                    fn route_name(self) -> &'static str {
                        match self {
                            #(#name_arms,)*
                        }
                    }
                }

                impl AppRoute {
                    /// 注册时的路径模板，如 `/profile/:id`。扫描不到则为 `None`。
                    pub fn uri(self) -> Option<&'static str> {
                        match self {
                            #(#uri_arms,)*
                        }
                    }

                    /// 填路径参数：`AppRoute::Profile.to(&[("id", "1")])` → `/profile/1`。
                    pub fn to(self, params: &[(&str, &str)]) -> Option<String> {
                        ::namix::routing::fill_uri(self.uri()?, params)
                    }

                    /// 无参 URL。路径含 `:id` 时请用 [`Self::to`]。
                    pub fn href(self) -> String {
                        self.to(&[])
                            .or_else(|| self.uri().map(str::to_string))
                            .unwrap_or_else(|| "/".into())
                    }
                }

                #(#consts)*
            }
        }
    });

    quote! {
        #(#apps)*
        #reexport
    }
    .into()
}

struct RouteNamesFile {
    apps: Vec<RouteNameApp>,
}

impl Parse for RouteNamesFile {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let mut apps = Vec::new();
        while !input.is_empty() {
            apps.push(input.parse()?);
        }
        Ok(Self { apps })
    }
}

struct RouteNameApp {
    name: Ident,
    leaves: Vec<RouteNameLeaf>,
}

impl Parse for RouteNameApp {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let name: Ident = input.parse()?;
        let content;
        syn::braced!(content in input);
        let mut leaves = Vec::new();
        while !content.is_empty() {
            leaves.push(content.parse()?);
            let _ = content.parse::<Token![,]>();
        }
        Ok(Self { name, leaves })
    }
}

struct RouteNameLeaf {
    ident: Ident,
    override_name: Option<LitStr>,
    uri: Option<LitStr>,
}

impl Parse for RouteNameLeaf {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let ident: Ident = input.parse()?;
        let override_name = if input.peek(Token![=]) {
            input.parse::<Token![=]>()?;
            Some(input.parse()?)
        } else {
            None
        };
        let uri = if input.peek(Token![=>]) {
            input.parse::<Token![=>]>()?;
            Some(input.parse()?)
        } else {
            None
        };
        Ok(Self {
            ident,
            override_name,
            uri,
        })
    }
}

fn enum_variant_ident(name: &str) -> String {
    let pascal = to_pascal_case(name);
    if pascal.is_empty() {
        "Route".into()
    } else if is_enum_keyword(&pascal) {
        format!("R{pascal}")
    } else {
        pascal
    }
}

fn is_enum_keyword(ident: &str) -> bool {
    matches!(ident, "Self" | "Crate")
        || matches!(
            ident,
            "as" | "async"
                | "await"
                | "break"
                | "const"
                | "continue"
                | "dyn"
                | "else"
                | "enum"
                | "extern"
                | "false"
                | "fn"
                | "for"
                | "if"
                | "impl"
                | "in"
                | "let"
                | "loop"
                | "match"
                | "mod"
                | "move"
                | "mut"
                | "pub"
                | "ref"
                | "return"
                | "static"
                | "struct"
                | "trait"
                | "true"
                | "type"
                | "unsafe"
                | "use"
                | "where"
                | "while"
        )
}

fn to_pascal_case(name: &str) -> String {
    name.split('_')
        .filter(|s| !s.is_empty())
        .map(|s| {
            let mut c = s.chars();
            match c.next() {
                None => String::new(),
                Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
            }
        })
        .collect()
}

fn derive_str_enum(
    input: TokenStream,
    attr_name: &str,
    trait_name: &str,
    method: &str,
    trait_path: &str,
) -> TokenStream {
    let input = parse_macro_input!(input as syn::DeriveInput);
    let enum_ident = &input.ident;
    let syn::Data::Enum(data) = &input.data else {
        return syn::Error::new_spanned(&input.ident, format!("{trait_name} only on enums"))
            .to_compile_error()
            .into();
    };

    let trait_path: syn::Path = syn::parse_str(trait_path).unwrap();
    let method_ident = Ident::new(method, proc_macro2::Span::call_site());

    let arms = data.variants.iter().map(|v| {
        let variant = &v.ident;
        let mut value = None;
        for attr in &v.attrs {
            if attr.path().is_ident(attr_name) {
                if let Ok(list) = attr.meta.require_list() {
                    // #[field("email")]
                    if let Ok(lit) = list.parse_args::<LitStr>() {
                        value = Some(lit.value());
                    }
                } else if let Ok(name_val) = attr.meta.require_name_value() {
                    // #[field = "email"]
                    if let syn::Expr::Lit(syn::ExprLit {
                        lit: syn::Lit::Str(s),
                        ..
                    }) = &name_val.value
                    {
                        value = Some(s.value());
                    }
                }
            }
        }
        let value = value.unwrap_or_else(|| to_snake_or_lower(&variant.to_string()));
        quote! {
            Self::#variant => #value,
        }
    });

    quote! {
        impl #trait_path for #enum_ident {
            fn #method_ident(self) -> &'static str {
                match self {
                    #(#arms)*
                }
            }
        }
    }
    .into()
}

fn to_snake_or_lower(name: &str) -> String {
    let mut out = String::new();
    for (i, ch) in name.chars().enumerate() {
        if ch.is_uppercase() {
            if i > 0 {
                out.push('_');
            }
            out.extend(ch.to_lowercase());
        } else {
            out.push(ch);
        }
    }
    out
}

fn to_shouty(name: &str) -> String {
    to_snake_or_lower(name).to_ascii_uppercase()
}

fn to_camel(name: &str) -> String {
    let mut out = String::new();
    let mut up = false;
    for (i, ch) in name.chars().enumerate() {
        if ch == '_' {
            up = true;
            continue;
        }
        if up {
            out.extend(ch.to_uppercase());
            up = false;
        } else if i == 0 {
            out.extend(ch.to_lowercase());
        } else {
            out.push(ch);
        }
    }
    out
}

fn serde_rename_all(attrs: &[syn::Attribute]) -> Option<String> {
    for attr in attrs {
        if !attr.path().is_ident("serde") {
            continue;
        }
        let mut found = None;
        let parser = syn::meta::parser(|meta| {
            if meta.path.is_ident("rename_all") {
                let lit: LitStr = meta.value()?.parse()?;
                found = Some(lit.value());
            } else {
                // 忽略其它 serde 键
                if meta.input.peek(Token![=]) {
                    let _: syn::Expr = meta.value()?.parse()?;
                }
            }
            Ok(())
        });
        let _ = attr.parse_args_with(parser);
        if found.is_some() {
            return found;
        }
    }
    None
}

fn serde_field_name(field: &syn::Field, ident: &Ident, rename_all: Option<&str>) -> String {
    for attr in &field.attrs {
        if !attr.path().is_ident("serde") {
            continue;
        }
        let mut rename = None;
        let parser = syn::meta::parser(|meta| {
            if meta.path.is_ident("rename") {
                let lit: LitStr = meta.value()?.parse()?;
                rename = Some(lit.value());
            } else if meta.input.peek(Token![=]) {
                let _: syn::Expr = meta.value()?.parse()?;
            }
            Ok(())
        });
        let _ = attr.parse_args_with(parser);
        if let Some(r) = rename {
            return r;
        }
    }
    let raw = ident.to_string();
    match rename_all {
        Some("camelCase") => to_camel(&raw),
        Some("snake_case") => to_snake_or_lower(&raw),
        _ => raw,
    }
}

struct TsType {
    ts: String,
    imports: Vec<String>,
}

fn rust_type_to_ts(ty: &syn::Type) -> TsType {
    let syn::Type::Path(p) = ty else {
        return TsType {
            ts: "unknown".into(),
            imports: vec![],
        };
    };
    let Some(seg) = p.path.segments.last() else {
        return TsType {
            ts: "unknown".into(),
            imports: vec![],
        };
    };
    let name = seg.ident.to_string();
    match name.as_str() {
        "String" | "str" => TsType {
            ts: "string".into(),
            imports: vec![],
        },
        "bool" => TsType {
            ts: "boolean".into(),
            imports: vec![],
        },
        "u8" | "u16" | "u32" | "u64" | "u128" | "usize" | "i8" | "i16" | "i32" | "i64" | "i128"
        | "isize" | "f32" | "f64" => TsType {
            ts: "number".into(),
            imports: vec![],
        },
        "Option" => {
            let inner = first_generic(seg).unwrap_or(TsType {
                ts: "unknown".into(),
                imports: vec![],
            });
            TsType {
                ts: format!("{} | null", inner.ts),
                imports: inner.imports,
            }
        }
        "Vec" | "VecDeque" => {
            let inner = first_generic(seg).unwrap_or(TsType {
                ts: "unknown".into(),
                imports: vec![],
            });
            TsType {
                ts: format!("{}[]", inner.ts),
                imports: inner.imports,
            }
        }
        // 自定义类型：与 #[derive(ViewData)] 生成的同名 TS 对齐
        other => TsType {
            ts: other.to_string(),
            imports: vec![other.to_string()],
        },
    }
}

fn first_generic(seg: &syn::PathSegment) -> Option<TsType> {
    let syn::PathArguments::AngleBracketed(args) = &seg.arguments else {
        return None;
    };
    for arg in &args.args {
        if let syn::GenericArgument::Type(ty) = arg {
            return Some(rust_type_to_ts(ty));
        }
    }
    None
}

/// 嵌套页面数据类型 → 独立 TS interface（供 ViewProps / TSX 补全）。
///
/// ```ignore
/// #[derive(Serialize, ViewData)]
/// #[serde(rename_all = "camelCase")]
/// pub struct DemoItem { pub id: u32, pub title: String }
/// ```
#[proc_macro_derive(ViewData)]
pub fn derive_view_data(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    let struct_ident = &input.ident;
    let Data::Struct(data) = &input.data else {
        return syn::Error::new_spanned(&input.ident, "ViewData only on structs")
            .to_compile_error()
            .into();
    };
    let Fields::Named(fields) = &data.fields else {
        return syn::Error::new_spanned(&input.ident, "ViewData needs named fields")
            .to_compile_error()
            .into();
    };

    let rename_all = serde_rename_all(&input.attrs);
    let mut ts_fields = Vec::new();
    let mut ts_imports: Vec<String> = Vec::new();
    for field in &fields.named {
        let name = field.ident.as_ref().unwrap();
        let json_name = serde_field_name(field, name, rename_all.as_deref());
        let mapped = rust_type_to_ts(&field.ty);
        for dep in &mapped.imports {
            if dep != &struct_ident.to_string() && !ts_imports.iter().any(|x| x == dep) {
                ts_imports.push(dep.clone());
            }
        }
        ts_fields.push(format!("  {json_name}: {};", mapped.ts));
    }

    let ty_name = struct_ident.to_string();
    let import_lines: String = ts_imports
        .iter()
        .map(|t| format!("import type {{ {t} }} from './{t}';\n"))
        .collect();
    let ts_body = format!(
        "/* @generated by #[derive(ViewData)] — DO NOT EDIT */\n\
         {imports}\
         export type {ty_name} = {{\n{fields}\n}};\n",
        imports = import_lines,
        ty_name = ty_name,
        fields = ts_fields.join("\n"),
    );
    write_generated_ts(&ty_name, &ts_body);

    // 纯 TS 契约，Rust 侧无额外代码
    quote! {}.into()
}

fn write_generated_ts(stem: &str, body: &str) {
    let Ok(manifest) = std::env::var("CARGO_MANIFEST_DIR") else {
        return;
    };
    let dir = std::path::PathBuf::from(manifest).join("src/views/generated");
    let _ = std::fs::create_dir_all(&dir);
    let path = dir.join(format!("{stem}.ts"));
    if std::fs::read_to_string(&path).ok().as_deref() != Some(body) {
        let _ = std::fs::write(path, body);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use syn::parse_quote;

    #[test]
    fn request_only_server_action_has_optional_browser_input() {
        let request: FnArg = parse_quote!(req: Request);
        let inputs = [&request];
        let mode = server_action_input_mode(&inputs);
        assert_eq!(mode, ServerActionInputMode::Optional);

        let ts = render_server_action_ts("submit", "deadbeef", mode);
        assert!(ts.contains("function submit(input?: Record<string, unknown>)"));
        assert!(ts.contains("callRust('deadbeef', input)"));
    }

    #[test]
    fn no_arg_and_dto_actions_keep_distinct_ts_arities() {
        let dto: FnArg = parse_quote!(input: LoginInput);
        let dto_inputs = [&dto];

        let no_input = render_server_action_ts("ping", "00000000", server_action_input_mode(&[]));
        let required =
            render_server_action_ts("login", "11111111", server_action_input_mode(&dto_inputs));

        assert!(no_input.contains("function ping()"));
        assert!(no_input.contains("callRust('00000000')"));
        assert!(required.contains("function login(input: Record<string, unknown>)"));
    }

    #[test]
    fn action_names_are_safe_rust_ts_identifiers_and_filenames() {
        for name in ["login", "send_code_2", "_health"] {
            assert!(validate_action_name(name).is_ok(), "{name}");
        }
        for name in [
            "",
            "user.login",
            "user-login",
            "../login",
            "1login",
            "登录",
            "delete",
            "match",
            "gen",
            "eval",
            "arguments",
            "CON",
            "COM1",
        ] {
            assert!(validate_action_name(name).is_err(), "{name}");
        }
    }
}
