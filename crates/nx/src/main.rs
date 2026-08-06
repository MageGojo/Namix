//! Namix CLI — bubbletea-rs TUI + make / migrate / doctor / build / dev

mod build;
mod dev;
mod doctor;
mod export;
mod frontend;
mod make;
mod migrate;
mod project;
mod release;
mod scope;
mod template;
mod ui;

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{CommandFactory, Parser, Subcommand};
use clap_complete::{Shell, generate};

use crate::template::{DatabaseDriver, FrontendLang, Mode, ScaffoldConfig};
use crate::ui::{HomeAction, WizardPreset, run_home, run_new_wizard};

#[derive(Parser)]
#[command(
    name = "nx",
    about = "Namix framework CLI",
    version,
    propagate_version = true,
    after_help = "提示:\n  \
nx new demo --single --tsx --tailwind --git\n  \
nx make model Article -m\n  \
nx make validator Login\n  \
nx make validator Checkout --app user   # 多应用：端专属\n  \
nx make controller Home --app user\n  \
nx migrate generate|apply|snapshot|reset\n  \
nx seed\n  \
nx export routes\n  \
nx dev                   # 同时启动 Rust + Vite(React)\n  \
nx build                 # 前后端发布包 → dist/<version>/\n  \
nx build --ver 1.0.0\n  \
nx start -p 3000         # 生产启动 dist/current（共享 data/）\n  \
nx update --build        # 热更新：编新版 → 重叠接流 → 旧进程排水\n  \
nx stop / nx status\n  \
nx doctor [--check]\n  \
nx completion zsh > _nx"
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// 创建新的 Namix 业务项目（可省略名称，进入引导）
    New {
        name: Option<String>,
        #[arg(long, group = "mode")]
        multi: bool,
        #[arg(long, group = "mode")]
        single: bool,
        #[arg(long, group = "https_flag")]
        https: bool,
        #[arg(long, group = "https_flag")]
        no_https: bool,
        /// 数据库驱动：sqlite | mysql | postgresql | custom
        #[arg(long = "db", value_name = "DRIVER")]
        db: Option<String>,
        /// Vite + React + TypeScript（默认）
        #[arg(long, group = "frontend_lang")]
        tsx: bool,
        /// Vite + React（无 TypeScript）
        #[arg(long, group = "frontend_lang")]
        jsx: bool,
        /// 启用 Tailwind CSS（默认）
        #[arg(long, group = "tailwind_flag")]
        tailwind: bool,
        #[arg(long, group = "tailwind_flag")]
        no_tailwind: bool,
        /// git init（默认）
        #[arg(long, group = "git_flag")]
        git: bool,
        #[arg(long, group = "git_flag")]
        no_git: bool,
        #[arg(long)]
        path: Option<PathBuf>,
    },
    /// 生成代码骨架（model / validator / resource / policy / job 等）
    Make {
        #[command(subcommand)]
        cmd: MakeCmd,
    },
    /// 数据库迁移（封装 app --bin toasty）
    Migrate {
        #[command(subcommand)]
        cmd: MigrateCmd,
    },
    /// 跑种子数据
    Seed,
    /// 开发：Vite 前端 HMR + Rust cargo-watch 热重载
    Dev {
        /// Rust HTTP 端口
        #[arg(short = 'p', long, default_value_t = 3000)]
        port: u16,
        /// Vite 端口
        #[arg(long, default_value_t = 5173)]
        vite_port: u16,
        /// 额外开 HTTPS
        #[arg(long)]
        https: bool,
        /// 只起 Vite
        #[arg(long)]
        frontend_only: bool,
        /// 只起 Rust（用已构建的 public/build）
        #[arg(long)]
        backend_only: bool,
        /// 关闭后端热重载（不跑 cargo-watch）
        #[arg(long)]
        no_reload: bool,
    },
    /// 导出前端产物（routes.ts 等）
    Export {
        #[command(subcommand)]
        cmd: ExportCmd,
    },
    /// 发布编译：前端 + 后端最小体积，产物到根目录 `dist/<version>/`
    #[command(visible_alias = "compile")]
    Build {
        /// 指定版本（x.y.z）；不传则在 dist/VERSION 上自动递增
        #[arg(long = "ver", value_name = "X.Y.Z")]
        ver: Option<String>,
        /// 自动递增档位：major|minor|patch（默认 patch；与 --ver 同时时以 --ver 为准）
        #[arg(long, default_value = "patch")]
        bump: String,
        /// 跳过前端
        #[arg(long)]
        no_frontend: bool,
        /// 关闭 JS 混淆（仍 minify）
        #[arg(long)]
        no_obfuscate: bool,
        /// 只编前端并打包到 dist
        #[arg(long)]
        frontend_only: bool,
        /// 只编后端并打包到 dist
        #[arg(long)]
        backend_only: bool,
        /// 不向共享区播种 SQLite；共享区已有库永不覆盖
        #[arg(long)]
        no_db: bool,
    },
    /// 生产启动：`dist/current`（数据在 `dist/data/storage`）
    Start {
        #[arg(short = 'p', long, default_value_t = 3000)]
        port: u16,
        /// 绑定 0.0.0.0（局域网）
        #[arg(long)]
        lan: bool,
        #[arg(long)]
        https: bool,
        #[arg(long = "https-port")]
        https_port: Option<u16>,
        /// 前台运行（默认后台）
        #[arg(long)]
        foreground: bool,
    },
    /// 优雅停止生产进程（SIGTERM 排水）
    Stop,
    /// 查看 current / 共享数据 / 进程状态
    Status,
    /// 生产热更新：可选编译 → 切换 current → 新旧重叠接流 → 旧进程排水
    Update {
        /// 先执行 `nx build`（默认：只切换已有 LATEST 并重启）
        #[arg(long)]
        build: bool,
        #[arg(long = "ver", value_name = "X.Y.Z")]
        ver: Option<String>,
        #[arg(long, default_value = "patch")]
        bump: String,
        #[arg(long)]
        no_frontend: bool,
        #[arg(long)]
        no_obfuscate: bool,
        /// 热更新时只重编后端（前端产物已在 dist）
        #[arg(long)]
        backend_only: bool,
        #[arg(short = 'p', long, default_value_t = 3000)]
        port: u16,
        /// 绑定 0.0.0.0（局域网）
        #[arg(long)]
        lan: bool,
        #[arg(long)]
        https: bool,
        #[arg(long = "https-port")]
        https_port: Option<u16>,
        /// 只改 current 指针，不重启
        #[arg(long)]
        swap_only: bool,
    },
    /// 自检项目结构（单/多应用）
    Doctor {
        /// 额外执行 `cargo check -p app`
        #[arg(long)]
        check: bool,
    },
    /// `doctor` 别名
    Check {
        #[arg(long)]
        compile: bool,
    },
    /// 生成 shell 补全脚本
    Completion {
        #[arg(value_enum)]
        shell: Shell,
    },
}

#[derive(Subcommand)]
enum MakeCmd {
    /// 生成实体 Model（多应用固定写 common/models）
    Model {
        /// 名称：Article / user_profile
        name: String,
        /// 生成后立刻 `migrate generate`
        #[arg(short = 'm', long)]
        migration: bool,
        /// 多应用忽略（model 只能 common）；保留参数以免误用
        #[arg(long = "app", visible_alias = "scope")]
        app: Option<String>,
    },
    /// 生成基础表单验证器
    ///
    /// 多应用：默认 `common/validators`；`--app user` → `user/validators`
    Validator {
        /// 名称：Login → LoginForm / login_form.rs
        name: String,
        /// 目标：common（默认）| www | user | admin
        #[arg(long = "app", visible_alias = "scope")]
        app: Option<String>,
    },
    /// 生成控制器（多应用必须 `--app`）
    Controller {
        /// 名称：Home → home.rs
        name: String,
        #[arg(long = "app", visible_alias = "scope")]
        app: Option<String>,
    },
    /// 生成 CRUD 资源控制器与资源路由骨架
    Resource {
        name: String,
        #[arg(long = "app", visible_alias = "scope")]
        app: Option<String>,
    },
    /// 生成 Policy / Gate 授权骨架
    Policy { name: String },
    /// 生成异步 Job 骨架
    Job { name: String },
    /// 生成邮件模板骨架
    Mail { name: String },
    /// 生成通知骨架
    Notification { name: String },
    /// 生成路由/Action 测试骨架
    Test { name: String },
}

#[derive(Subcommand)]
enum MigrateCmd {
    /// 按当前 Model 生成迁移 SQL
    Generate,
    /// 应用待执行迁移
    Apply,
    /// 打印 schema snapshot
    Snapshot,
    /// 重置数据库（危险）
    Reset,
}

#[derive(Subcommand)]
enum ExportCmd {
    /// 从 storage/routes.json 生成 storage/routes.ts（Ziggy 风格 route()）
    Routes,
}

#[tokio::main]
async fn main() -> ExitCode {
    let cli = Cli::parse();

    match cli.command {
        None => match run_home().await {
            Ok(HomeAction::Quit) => ExitCode::SUCCESS,
            Ok(HomeAction::Help) => {
                let mut cmd = Cli::command();
                let _ = cmd.print_long_help();
                ExitCode::SUCCESS
            }
            Ok(HomeAction::New { name }) => {
                run_new(NewOpts {
                    name,
                    multi: false,
                    single: false,
                    https: None,
                    database: None,
                    frontend: None,
                    tailwind: None,
                    git: None,
                    path: None,
                })
                .await
            }
            Err(e) => {
                eprintln!("TUI 错误: {e}");
                ExitCode::FAILURE
            }
        },
        Some(Commands::Completion { shell }) => {
            let mut cmd = Cli::command();
            let mut out = std::io::stdout();
            generate(shell, &mut cmd, "nx", &mut out);
            let _ = out.flush();
            ExitCode::SUCCESS
        }
        Some(Commands::New {
            name,
            multi,
            single,
            https,
            no_https,
            db,
            tsx,
            jsx,
            tailwind,
            no_tailwind,
            git,
            no_git,
            path,
        }) => {
            let https = if no_https {
                Some(false)
            } else if https {
                Some(true)
            } else {
                None
            };
            let database = match db.as_deref() {
                None => None,
                Some(s) => match parse_db_driver(s) {
                    Ok(d) => Some(d),
                    Err(e) => {
                        eprintln!("{e}");
                        return ExitCode::FAILURE;
                    }
                },
            };
            let frontend = if tsx {
                Some(FrontendLang::Tsx)
            } else if jsx {
                Some(FrontendLang::Jsx)
            } else {
                None
            };
            let tailwind = if no_tailwind {
                Some(false)
            } else if tailwind {
                Some(true)
            } else {
                None
            };
            let git = if no_git {
                Some(false)
            } else if git {
                Some(true)
            } else {
                None
            };
            run_new(NewOpts {
                name,
                multi,
                single,
                https,
                database,
                frontend,
                tailwind,
                git,
                path,
            })
            .await
        }
        Some(Commands::Make { cmd }) => finish(run_make(cmd)),
        Some(Commands::Migrate { cmd }) => finish(run_migrate(cmd)),
        Some(Commands::Seed) => finish(run_seed()),
        Some(Commands::Dev {
            port,
            vite_port,
            https,
            frontend_only,
            backend_only,
            no_reload,
        }) => finish(run_dev(
            port,
            vite_port,
            https,
            frontend_only,
            backend_only,
            no_reload,
        )),
        Some(Commands::Export { cmd }) => finish(run_export(cmd)),
        Some(Commands::Build {
            ver,
            bump,
            no_frontend,
            no_obfuscate,
            frontend_only,
            backend_only,
            no_db,
        }) => finish(run_build(BuildCli {
            version: ver,
            bump,
            no_frontend,
            no_obfuscate,
            frontend_only,
            backend_only,
            no_db,
        })),
        Some(Commands::Start {
            port,
            lan,
            https,
            https_port,
            foreground,
        }) => finish(run_start(port, lan, https, https_port, foreground)),
        Some(Commands::Stop) => finish(run_stop()),
        Some(Commands::Status) => finish(run_status()),
        Some(Commands::Update {
            build,
            ver,
            bump,
            no_frontend,
            no_obfuscate,
            backend_only,
            port,
            lan,
            https,
            https_port,
            swap_only,
        }) => finish(run_update(UpdateCli {
            build,
            version: ver,
            bump,
            no_frontend,
            no_obfuscate,
            backend_only,
            port,
            lan,
            https,
            https_port,
            swap_only,
        })),
        Some(Commands::Doctor { check }) => finish(run_doctor(check)),
        Some(Commands::Check { compile }) => finish(run_doctor(compile)),
    }
}

struct UpdateCli {
    build: bool,
    version: Option<String>,
    bump: String,
    no_frontend: bool,
    no_obfuscate: bool,
    backend_only: bool,
    port: u16,
    lan: bool,
    https: bool,
    https_port: Option<u16>,
    swap_only: bool,
}

fn run_start(
    port: u16,
    lan: bool,
    https: bool,
    https_port: Option<u16>,
    foreground: bool,
) -> Result<(), String> {
    let project = project::discover()?;
    release::start(
        &project,
        release::StartOpts {
            port,
            lan,
            https,
            https_port,
            foreground,
        },
    )
}

fn run_stop() -> Result<(), String> {
    release::stop(&project::discover()?)
}

fn run_status() -> Result<(), String> {
    release::status(&project::discover()?)
}

fn run_update(cli: UpdateCli) -> Result<(), String> {
    let project = project::discover()?;
    let bump = release::parse_bump(&cli.bump)?;
    release::update(
        &project,
        release::UpdateOpts {
            build: cli.build,
            bump,
            version: cli.version,
            no_frontend: cli.no_frontend,
            no_obfuscate: cli.no_obfuscate,
            backend_only: cli.backend_only,
            port: cli.port,
            lan: cli.lan,
            https: cli.https,
            https_port: cli.https_port,
            swap_only: cli.swap_only,
        },
    )
}

struct BuildCli {
    version: Option<String>,
    bump: String,
    no_frontend: bool,
    no_obfuscate: bool,
    frontend_only: bool,
    backend_only: bool,
    no_db: bool,
}

fn run_build(cli: BuildCli) -> Result<(), String> {
    let project = project::discover()?;
    let bump = build::Bump::parse(&cli.bump)?;
    if cli.frontend_only && cli.backend_only {
        return Err("--frontend-only 与 --backend-only 不能同时用".into());
    }
    build::run(
        &project,
        build::BuildOpts {
            version: cli.version,
            bump,
            no_frontend: cli.no_frontend,
            no_obfuscate: cli.no_obfuscate,
            frontend_only: cli.frontend_only,
            backend_only: cli.backend_only,
            no_db: cli.no_db,
            activate: true,
        },
    )
}

fn run_export(cmd: ExportCmd) -> Result<(), String> {
    let project = project::discover()?;
    match cmd {
        ExportCmd::Routes => export::routes(&project),
    }
}

fn finish(r: Result<(), String>) -> ExitCode {
    match r {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("✗ {e}");
            ExitCode::FAILURE
        }
    }
}

fn run_make(cmd: MakeCmd) -> Result<(), String> {
    let project = project::discover()?;
    match cmd {
        MakeCmd::Model {
            name,
            migration,
            app,
        } => make::model(&project, &name, migration, app.as_deref()),
        MakeCmd::Validator { name, app } => make::validator(&project, &name, app.as_deref()),
        MakeCmd::Controller { name, app } => make::controller(&project, &name, app.as_deref()),
        MakeCmd::Resource { name, app } => make::resource(&project, &name, app.as_deref()),
        MakeCmd::Policy { name } => make::policy(&project, &name),
        MakeCmd::Job { name } => make::job(&project, &name),
        MakeCmd::Mail { name } => make::mail(&project, &name),
        MakeCmd::Notification { name } => make::notification(&project, &name),
        MakeCmd::Test { name } => make::test(&project, &name),
    }
}

fn run_migrate(cmd: MigrateCmd) -> Result<(), String> {
    let project = project::discover()?;
    match cmd {
        MigrateCmd::Generate => migrate::generate(&project),
        MigrateCmd::Apply => migrate::apply(&project),
        MigrateCmd::Snapshot => migrate::status(&project),
        MigrateCmd::Reset => migrate::reset(&project),
    }
}

fn run_seed() -> Result<(), String> {
    let project = project::discover()?;
    migrate::seed(&project)
}

fn run_dev(
    port: u16,
    vite_port: u16,
    https: bool,
    frontend_only: bool,
    backend_only: bool,
    no_reload: bool,
) -> Result<(), String> {
    if frontend_only && backend_only {
        return Err("--frontend-only 与 --backend-only 不能同时用".into());
    }
    let project = project::discover()?;
    dev::run(
        &project,
        dev::DevOpts {
            port,
            vite_port,
            https,
            frontend_only,
            backend_only,
            no_reload,
        },
    )
}

fn run_doctor(with_compile: bool) -> Result<(), String> {
    let project = project::discover()?;
    doctor::run(&project, with_compile)
}

struct NewOpts {
    name: Option<String>,
    multi: bool,
    single: bool,
    https: Option<bool>,
    database: Option<DatabaseDriver>,
    frontend: Option<FrontendLang>,
    tailwind: Option<bool>,
    git: Option<bool>,
    path: Option<PathBuf>,
}

fn parse_db_driver(s: &str) -> Result<DatabaseDriver, String> {
    match s.trim().to_ascii_lowercase().as_str() {
        "sqlite" | "sqlite3" => Ok(DatabaseDriver::Sqlite),
        "mysql" | "mariadb" => Ok(DatabaseDriver::Mysql),
        "postgresql" | "postgres" | "pgsql" => Ok(DatabaseDriver::Postgresql),
        "custom" => Ok(DatabaseDriver::Custom),
        other => Err(format!(
            "未知 --db={other}，可选：sqlite | mysql | postgresql | custom"
        )),
    }
}

async fn run_new(opts: NewOpts) -> ExitCode {
    let mode = if opts.multi {
        Some(Mode::Multi)
    } else if opts.single {
        Some(Mode::Single)
    } else {
        None
    };

    // name+mode+https 齐备：无 TTY 也可直接创建（默认 sqlite / tsx / tailwind / git）
    if let (Some(name), Some(mode), Some(https)) = (&opts.name, mode, opts.https) {
        let database = opts.database.unwrap_or(DatabaseDriver::Sqlite);
        let frontend = opts.frontend.unwrap_or(FrontendLang::Tsx);
        let tailwind = opts.tailwind.unwrap_or(true);
        let git = opts.git.unwrap_or(true);
        let path = opts.path.clone().unwrap_or_else(|| PathBuf::from(name));
        let cfg = ScaffoldConfig {
            mode,
            https,
            database,
            frontend,
            tailwind,
            git,
        };
        return match template::scaffold(&path, name, &cfg) {
            Ok(()) => {
                print_new_success(name, &path, &cfg);
                ExitCode::SUCCESS
            }
            Err(e) => {
                eprintln!("创建失败: {e}");
                ExitCode::FAILURE
            }
        };
    }

    let result = match run_new_wizard(WizardPreset {
        name: opts.name,
        mode,
        https: opts.https,
        database: opts.database,
        frontend: opts.frontend,
        tailwind: opts.tailwind,
        git: opts.git,
        path: opts.path,
    })
    .await
    {
        Ok(r) => r,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::FAILURE;
        }
    };

    if result.cancelled {
        println!("已取消");
        return ExitCode::SUCCESS;
    }

    if let Some(e) = result.error {
        eprintln!("创建失败: {e}");
        return ExitCode::FAILURE;
    }

    print_new_success(
        &result.name,
        &result.path,
        &ScaffoldConfig {
            mode: result.mode,
            https: result.https,
            database: result.database,
            frontend: result.frontend,
            tailwind: result.tailwind,
            git: result.git,
        },
    );
    ExitCode::SUCCESS
}

fn print_new_success(name: &str, path: &Path, cfg: &ScaffoldConfig) {
    println!();
    println!("✓ {name} 已创建 → {}", path.display());
    println!(
        "  https={}  database={}  frontend={}  tailwind={}  git={}",
        if cfg.https { "on" } else { "off" },
        cfg.database.label(),
        cfg.frontend.label(),
        if cfg.tailwind { "on" } else { "off" },
        if cfg.git { "on" } else { "off" },
    );
    println!("  cd {}", path.display());
    match cfg.mode {
        Mode::Multi => {
            println!("  cargo run -p app --bin www -- -p 3000 -h");
            println!("  cargo run -p app --bin user");
            println!("  cargo run -p app --bin admin");
        }
        Mode::Single => {
            println!("  nx doctor");
            println!("  nx seed");
            println!("  cargo run -p app -- -p 3000 -h");
        }
    }
    println!("  cd frontend && npm i && npm run dev");
    if !cfg.https {
        println!("  需要 HTTPS 时追加 --https");
    }
    println!("  生成: nx make model Foo -m · nx make validator Login");
    println!("  迁移: nx migrate generate|apply · 路由: nx export routes");
}
