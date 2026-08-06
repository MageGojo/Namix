//! `nx new` 向导 + 创建进度动画（bubbletea-widgets progress）。

use std::path::PathBuf;
use std::sync::OnceLock;
use std::time::Duration;

use bubbletea_rs::{
    Cmd, KeyMsg, Model, Msg, Program, WindowSizeMsg, batch, quit, tick, window_size,
};
use bubbletea_widgets::progress::{self, FrameMsg, with_gradient, with_spring_options, with_width};
use bubbletea_widgets::textinput;
use crossterm::event::{KeyCode, KeyModifiers};
use lipgloss_extras::lipgloss::{Style, color::TEXT_SUBTLE};

use crate::template::{self, DatabaseDriver, FrontendLang, Mode, ScaffoldConfig};

use super::theme;

static PRESET: OnceLock<WizardPreset> = OnceLock::new();

const PAD: usize = 2;
const MAX_WIDTH: i32 = 80;
const STEPS: &[&str] = &[
    "name", "mode", "https", "database", "frontend", "tailwind", "git", "confirm", "create",
];

const CREATE_JOBS: &[&str] = &[
    "准备目录",
    "写入 workspace",
    "生成 app 结构",
    "生成 frontend",
    "git init",
    "完成",
];

#[derive(Clone, Debug)]
pub struct WizardPreset {
    pub name: Option<String>,
    pub mode: Option<Mode>,
    pub https: Option<bool>,
    pub database: Option<DatabaseDriver>,
    pub frontend: Option<FrontendLang>,
    pub tailwind: Option<bool>,
    pub git: Option<bool>,
    pub path: Option<PathBuf>,
}

#[derive(Clone, Debug)]
pub struct NewWizardResult {
    pub name: String,
    pub mode: Mode,
    pub https: bool,
    pub database: DatabaseDriver,
    pub frontend: FrontendLang,
    pub tailwind: bool,
    pub git: bool,
    pub path: PathBuf,
    pub cancelled: bool,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Step {
    Name,
    Mode,
    Https,
    Database,
    Frontend,
    Tailwind,
    Git,
    Confirm,
    Creating,
    Done,
}

struct TickMsg;

pub struct NewWizard {
    step: Step,
    name_input: textinput::Model,
    mode_idx: usize,
    https_idx: usize,
    db_idx: usize,
    frontend_idx: usize,
    tailwind_idx: usize,
    git_idx: usize,
    path: Option<PathBuf>,
    error: Option<String>,
    result: Option<NewWizardResult>,
    cancelled: bool,
    progress: progress::Model,
    create_stage: usize,
    create_done: bool,
    scaffolded: bool,
    width: i32,
}

const MODES: &[(&str, &str, Mode)] = &[
    (
        "多应用 Multi",
        "www / user / admin + common（推荐）",
        Mode::Multi,
    ),
    (
        "单应用 Single",
        "一个入口，适合简单站点 / API",
        Mode::Single,
    ),
];

const HTTPS_OPTS: &[(&str, &str, bool)] = &[
    (
        "启用本地 HTTPS",
        "namix.toml → https=true（自签证书）",
        true,
    ),
    ("仅 HTTP", "https=false，需要时再用 --https", false),
];

const DB_OPTS: &[(&str, &str, DatabaseDriver)] = &[
    (
        "SQLite",
        "文件库 sqlite:./storage/namix.db（默认，零依赖）",
        DatabaseDriver::Sqlite,
    ),
    (
        "MySQL",
        "mysql://root@127.0.0.1:3306/namix · Cargo feature=mysql",
        DatabaseDriver::Mysql,
    ),
    (
        "PostgreSQL",
        "postgresql://postgres@127.0.0.1:5432/namix · feature=postgresql",
        DatabaseDriver::Postgresql,
    ),
    (
        "Custom",
        "自填连接串（改 namix.toml url + Cargo features）",
        DatabaseDriver::Custom,
    ),
];

const FRONTEND_OPTS: &[(&str, &str, FrontendLang)] = &[
    (
        "TypeScript TSX",
        "Vite + React + TypeScript（默认）",
        FrontendLang::Tsx,
    ),
    (
        "JavaScript JSX",
        "Vite + React，无 TypeScript",
        FrontendLang::Jsx,
    ),
];

const TAILWIND_OPTS: &[(&str, &str, bool)] = &[
    (
        "启用 Tailwind CSS",
        "frontend 使用 Tailwind v4（默认）",
        true,
    ),
    ("不用 Tailwind", "只生成基础 CSS", false),
];

const GIT_OPTS: &[(&str, &str, bool)] = &[
    ("初始化 Git", "在项目根执行 git init（默认）", true),
    ("跳过 Git", "不运行 git init", false),
];

impl NewWizard {
    fn style_input(input: &mut textinput::Model) {
        input.prompt = "> ".into();
        input.prompt_style = Style::new().foreground(theme::BRAND);
        input.placeholder_style = Style::new().foreground(TEXT_SUBTLE);
        input.text_style = Style::new().foreground(theme::BRAND);
        input.completion_style = Style::new().foreground(TEXT_SUBTLE);
        input.set_placeholder("my-app");
        input.set_char_limit(64);
        input.set_width(36);
        input.set_suggestions(vec![
            "demo".into(),
            "blog".into(),
            "shop".into(),
            "admin-portal".into(),
            "namix-app".into(),
        ]);
    }

    fn new_progress() -> progress::Model {
        progress::new(&[
            with_gradient(theme::GRADIENT_A.into(), theme::GRADIENT_B.into()),
            with_width(46),
            with_spring_options(18.0, 1.2),
        ])
    }

    fn from_preset(preset: &WizardPreset) -> (Self, Option<Cmd>) {
        let mut name_input = textinput::new();
        Self::style_input(&mut name_input);

        let mode_idx = preset
            .mode
            .and_then(|m| MODES.iter().position(|o| o.2 == m))
            .unwrap_or(0);
        let https_idx = preset
            .https
            .and_then(|v| HTTPS_OPTS.iter().position(|o| o.2 == v))
            .unwrap_or(0);
        let db_idx = preset
            .database
            .and_then(|v| DB_OPTS.iter().position(|o| o.2 == v))
            .unwrap_or(0);
        let frontend_idx = preset
            .frontend
            .and_then(|v| FRONTEND_OPTS.iter().position(|o| o.2 == v))
            .unwrap_or(0);
        let tailwind_idx = preset
            .tailwind
            .and_then(|v| TAILWIND_OPTS.iter().position(|o| o.2 == v))
            .unwrap_or(0);
        let git_idx = preset
            .git
            .and_then(|v| GIT_OPTS.iter().position(|o| o.2 == v))
            .unwrap_or(0);

        if let Some(name) = &preset.name {
            name_input.set_value(name);
        }

        // 兼容旧 CLI：name+mode+https 齐则直进创建（frontend/tailwind/git 用默认或 flag）
        let step = if preset.name.is_some() && preset.mode.is_some() && preset.https.is_some() {
            Step::Creating
        } else if preset.name.is_none() {
            Step::Name
        } else if preset.mode.is_none() {
            Step::Mode
        } else if preset.https.is_none() {
            Step::Https
        } else if preset.database.is_none() {
            Step::Database
        } else if preset.frontend.is_none() {
            Step::Frontend
        } else if preset.tailwind.is_none() {
            Step::Tailwind
        } else if preset.git.is_none() {
            Step::Git
        } else {
            Step::Confirm
        };

        let mut model = Self {
            step,
            name_input,
            mode_idx,
            https_idx,
            db_idx,
            frontend_idx,
            tailwind_idx,
            git_idx,
            path: preset.path.clone(),
            error: None,
            result: None,
            cancelled: false,
            progress: Self::new_progress(),
            create_stage: 0,
            create_done: false,
            scaffolded: false,
            width: 60,
        };

        let cmd = match model.step {
            Step::Name => Some(batch(vec![window_size(), model.name_input.focus()])),
            Step::Creating => Some(batch(vec![window_size(), tick_cmd()])),
            _ => Some(window_size()),
        };

        (model, cmd)
    }

    fn selected_mode(&self) -> Mode {
        MODES[self.mode_idx].2
    }

    fn selected_https(&self) -> bool {
        HTTPS_OPTS[self.https_idx].2
    }

    fn selected_database(&self) -> DatabaseDriver {
        DB_OPTS[self.db_idx].2
    }

    fn selected_frontend(&self) -> FrontendLang {
        FRONTEND_OPTS[self.frontend_idx].2
    }

    fn selected_tailwind(&self) -> bool {
        TAILWIND_OPTS[self.tailwind_idx].2
    }

    fn selected_git(&self) -> bool {
        GIT_OPTS[self.git_idx].2
    }

    fn project_name(&self) -> String {
        self.name_input.value().trim().to_string()
    }

    fn project_path(&self) -> PathBuf {
        self.path
            .clone()
            .unwrap_or_else(|| PathBuf::from(self.project_name()))
    }

    fn validate_name(name: &str) -> Result<(), String> {
        if name.is_empty() {
            return Err("项目名称不能为空".into());
        }
        if !name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        {
            return Err("名称只能包含字母、数字、'-'、'_'".into());
        }
        Ok(())
    }

    fn step_index(&self) -> usize {
        match self.step {
            Step::Name => 0,
            Step::Mode => 1,
            Step::Https => 2,
            Step::Database => 3,
            Step::Frontend => 4,
            Step::Tailwind => 5,
            Step::Git => 6,
            Step::Confirm => 7,
            Step::Creating | Step::Done => 8,
        }
    }

    fn start_creating(&mut self) -> Option<Cmd> {
        self.error = None;
        self.step = Step::Creating;
        self.create_stage = 0;
        self.create_done = false;
        self.scaffolded = false;
        self.progress = Self::new_progress();
        self.progress.width = (self.width - (PAD as i32) * 2 - 4).clamp(20, MAX_WIDTH);
        Some(batch(vec![self.progress.set_percent(0.05), tick_cmd()]))
    }

    fn result_base(&self, cancelled: bool, error: Option<String>) -> NewWizardResult {
        NewWizardResult {
            name: self.project_name(),
            mode: self.selected_mode(),
            https: self.selected_https(),
            database: self.selected_database(),
            frontend: self.selected_frontend(),
            tailwind: self.selected_tailwind(),
            git: self.selected_git(),
            path: self.project_path(),
            cancelled,
            error,
        }
    }

    fn finish_ok(&mut self) -> Option<Cmd> {
        self.step = Step::Done;
        self.create_done = true;
        self.result = Some(self.result_base(false, None));
        Some(quit())
    }

    fn finish_err(&mut self, e: String) -> Option<Cmd> {
        self.error = Some(e.clone());
        self.result = Some(self.result_base(false, Some(e)));
        self.step = Step::Done;
        Some(quit())
    }

    fn advance_create(&mut self) -> Option<Cmd> {
        if !self.scaffolded && self.create_stage >= 2 {
            let cfg = ScaffoldConfig {
                mode: self.selected_mode(),
                https: self.selected_https(),
                database: self.selected_database(),
                frontend: self.selected_frontend(),
                tailwind: self.selected_tailwind(),
                git: self.selected_git(),
            };
            match template::scaffold(&self.project_path(), &self.project_name(), &cfg) {
                Ok(()) => self.scaffolded = true,
                Err(e) => return self.finish_err(e),
            }
        }

        if self.create_stage + 1 >= CREATE_JOBS.len() {
            let anim = self.progress.set_percent(1.0);
            self.create_stage = CREATE_JOBS.len() - 1;
            self.create_done = true;
            return Some(batch(vec![anim, settle_tick()]));
        }

        self.create_stage += 1;
        let target = (self.create_stage as f64) / ((CREATE_JOBS.len() - 1) as f64);
        let anim = self.progress.set_percent(target.clamp(0.0, 1.0));
        Some(batch(vec![anim, tick_cmd()]))
    }

    fn choice_view(
        &self,
        title: &str,
        subtitle: &str,
        opts: &[(&str, &str)],
        idx: usize,
    ) -> String {
        let pad = " ".repeat(PAD);
        let mut s = format!("{}\n{}\n\n", theme::header(title), theme::muted(subtitle));
        for (i, (label, desc)) in opts.iter().enumerate() {
            if i > 0 {
                s.push('\n');
            }
            s.push_str(&format!(
                "{}{}\n",
                pad,
                theme::select_line(i == idx, label, desc)
            ));
        }
        s.push_str(&format!(
            "\n{}{}\n",
            pad,
            theme::key_hint(&[("↑↓", "select"), ("enter", "next"), ("backspace", "back")])
        ));
        s
    }
}

fn tick_cmd() -> Cmd {
    tick(Duration::from_millis(420), |_| Box::new(TickMsg) as Msg)
}

fn settle_tick() -> Cmd {
    tick(Duration::from_millis(90), |_| Box::new(TickMsg) as Msg)
}

impl Model for NewWizard {
    fn init() -> (Self, Option<Cmd>) {
        let preset = PRESET.get().cloned().unwrap_or(WizardPreset {
            name: None,
            mode: None,
            https: None,
            database: None,
            frontend: None,
            tailwind: None,
            git: None,
            path: None,
        });
        Self::from_preset(&preset)
    }

    fn update(&mut self, msg: Msg) -> Option<Cmd> {
        if msg.downcast_ref::<FrameMsg>().is_some() {
            return self.progress.update(msg);
        }

        if let Some(size) = msg.downcast_ref::<WindowSizeMsg>() {
            self.width = size.width as i32;
            let w = (size.width as i32) - (PAD as i32) * 2 - 4;
            self.progress.width = w.clamp(20, MAX_WIDTH);
            return None;
        }

        if msg.downcast_ref::<TickMsg>().is_some() {
            if self.step != Step::Creating {
                return None;
            }
            if self.create_done {
                if self.progress.percent() >= 0.995 {
                    return self.finish_ok();
                }
                return Some(settle_tick());
            }
            return self.advance_create();
        }

        if let Some(key) = msg.downcast_ref::<KeyMsg>() {
            if (key.modifiers.contains(KeyModifiers::CONTROL) && key.key == KeyCode::Char('c'))
                || (key.key == KeyCode::Esc && self.step != Step::Creating)
            {
                self.cancelled = true;
                self.result = Some(NewWizardResult {
                    name: String::new(),
                    mode: Mode::Multi,
                    https: true,
                    database: DatabaseDriver::Sqlite,
                    frontend: FrontendLang::Tsx,
                    tailwind: true,
                    git: true,
                    path: PathBuf::new(),
                    cancelled: true,
                    error: None,
                });
                return Some(quit());
            }

            if self.step == Step::Creating {
                return None;
            }

            match self.step {
                Step::Name => {
                    if key.key == KeyCode::Enter {
                        match Self::validate_name(&self.project_name()) {
                            Ok(()) => {
                                self.error = None;
                                self.step = Step::Mode;
                            }
                            Err(e) => self.error = Some(e),
                        }
                        return None;
                    }
                    return self.name_input.update(msg);
                }
                Step::Mode => match key.key {
                    KeyCode::Up | KeyCode::Char('k') => {
                        if self.mode_idx > 0 {
                            self.mode_idx -= 1;
                        }
                    }
                    KeyCode::Down | KeyCode::Char('j') => {
                        if self.mode_idx + 1 < MODES.len() {
                            self.mode_idx += 1;
                        }
                    }
                    KeyCode::Enter => self.step = Step::Https,
                    KeyCode::Backspace => {
                        self.step = Step::Name;
                        return Some(self.name_input.focus());
                    }
                    _ => {}
                },
                Step::Https => match key.key {
                    KeyCode::Up | KeyCode::Char('k') => {
                        if self.https_idx > 0 {
                            self.https_idx -= 1;
                        }
                    }
                    KeyCode::Down | KeyCode::Char('j') => {
                        if self.https_idx + 1 < HTTPS_OPTS.len() {
                            self.https_idx += 1;
                        }
                    }
                    KeyCode::Enter => self.step = Step::Database,
                    KeyCode::Backspace => self.step = Step::Mode,
                    _ => {}
                },
                Step::Database => match key.key {
                    KeyCode::Up | KeyCode::Char('k') => {
                        if self.db_idx > 0 {
                            self.db_idx -= 1;
                        }
                    }
                    KeyCode::Down | KeyCode::Char('j') => {
                        if self.db_idx + 1 < DB_OPTS.len() {
                            self.db_idx += 1;
                        }
                    }
                    KeyCode::Enter => self.step = Step::Frontend,
                    KeyCode::Backspace => self.step = Step::Https,
                    _ => {}
                },
                Step::Frontend => match key.key {
                    KeyCode::Up | KeyCode::Char('k') => {
                        if self.frontend_idx > 0 {
                            self.frontend_idx -= 1;
                        }
                    }
                    KeyCode::Down | KeyCode::Char('j') => {
                        if self.frontend_idx + 1 < FRONTEND_OPTS.len() {
                            self.frontend_idx += 1;
                        }
                    }
                    KeyCode::Enter => self.step = Step::Tailwind,
                    KeyCode::Backspace => self.step = Step::Database,
                    _ => {}
                },
                Step::Tailwind => match key.key {
                    KeyCode::Up | KeyCode::Char('k') => {
                        if self.tailwind_idx > 0 {
                            self.tailwind_idx -= 1;
                        }
                    }
                    KeyCode::Down | KeyCode::Char('j') => {
                        if self.tailwind_idx + 1 < TAILWIND_OPTS.len() {
                            self.tailwind_idx += 1;
                        }
                    }
                    KeyCode::Enter => self.step = Step::Git,
                    KeyCode::Backspace => self.step = Step::Frontend,
                    _ => {}
                },
                Step::Git => match key.key {
                    KeyCode::Up | KeyCode::Char('k') => {
                        if self.git_idx > 0 {
                            self.git_idx -= 1;
                        }
                    }
                    KeyCode::Down | KeyCode::Char('j') => {
                        if self.git_idx + 1 < GIT_OPTS.len() {
                            self.git_idx += 1;
                        }
                    }
                    KeyCode::Enter => self.step = Step::Confirm,
                    KeyCode::Backspace => self.step = Step::Tailwind,
                    _ => {}
                },
                Step::Confirm => match key.key {
                    KeyCode::Enter | KeyCode::Char('y') => return self.start_creating(),
                    KeyCode::Char('n') | KeyCode::Backspace => self.step = Step::Git,
                    _ => {}
                },
                Step::Creating | Step::Done => {}
            }
        } else if self.step == Step::Name {
            return self.name_input.update(msg);
        }
        None
    }

    fn view(&self) -> String {
        let pad = " ".repeat(PAD);
        let steps = theme::step_dots(self.step_index(), STEPS);

        let body = match self.step {
            Step::Name => {
                let mut s = format!(
                    "{}\n\n{}{}\n",
                    theme::header("Project name"),
                    pad,
                    self.name_input.view()
                );
                if let Some(err) = &self.error {
                    s.push_str(&format!("\n{}{}\n", pad, theme::err(err)));
                } else {
                    s.push_str(&format!(
                        "\n{}{}\n",
                        pad,
                        theme::help("suggestions: demo · blog · shop  ·  tab to complete")
                    ));
                }
                s.push_str(&format!(
                    "\n{}{}\n",
                    pad,
                    theme::key_hint(&[("enter", "next"), ("esc", "cancel")])
                ));
                s
            }
            Step::Mode => {
                let opts: Vec<_> = MODES.iter().map(|(a, b, _)| (*a, *b)).collect();
                self.choice_view(
                    "Application mode",
                    &format!("name = {}", self.project_name()),
                    &opts,
                    self.mode_idx,
                )
            }
            Step::Https => {
                let opts: Vec<_> = HTTPS_OPTS.iter().map(|(a, b, _)| (*a, *b)).collect();
                self.choice_view(
                    "Local HTTPS",
                    "写入每个 [apps.*] 的 https 开关",
                    &opts,
                    self.https_idx,
                )
            }
            Step::Database => {
                let opts: Vec<_> = DB_OPTS.iter().map(|(a, b, _)| (*a, *b)).collect();
                self.choice_view(
                    "Database",
                    "写入 [database] driver/url，并设置 namix/toasty Cargo feature",
                    &opts,
                    self.db_idx,
                )
            }
            Step::Frontend => {
                let opts: Vec<_> = FRONTEND_OPTS.iter().map(|(a, b, _)| (*a, *b)).collect();
                self.choice_view(
                    "Frontend language",
                    "生成 Vite + React 项目（frontend/）",
                    &opts,
                    self.frontend_idx,
                )
            }
            Step::Tailwind => {
                let opts: Vec<_> = TAILWIND_OPTS.iter().map(|(a, b, _)| (*a, *b)).collect();
                self.choice_view("Tailwind CSS", "样式方案", &opts, self.tailwind_idx)
            }
            Step::Git => {
                let opts: Vec<_> = GIT_OPTS.iter().map(|(a, b, _)| (*a, *b)).collect();
                self.choice_view("Git", "是否在项目根执行 git init", &opts, self.git_idx)
            }
            Step::Confirm => {
                let mode = MODES[self.mode_idx];
                let https = if self.selected_https() { "on" } else { "off" };
                let db = self.selected_database().label();
                let fe = self.selected_frontend().label();
                let tw = if self.selected_tailwind() {
                    "on"
                } else {
                    "off"
                };
                let git = if self.selected_git() { "on" } else { "off" };
                format!(
                    "{}\n\n{}{}  {}\n{}{}  {}\n{}{}  {}\n{}{}  {}\n{}{}  {}\n{}{}  {}\n{}{}  {}\n{}{}  {}\n\n{}{}\n",
                    theme::header("Ready to create"),
                    pad,
                    theme::muted("name"),
                    theme::brand(&self.project_name()),
                    pad,
                    theme::muted("mode"),
                    theme::brand(mode.0),
                    pad,
                    theme::muted("https"),
                    theme::brand(https),
                    pad,
                    theme::muted("database"),
                    theme::brand(db),
                    pad,
                    theme::muted("frontend"),
                    theme::brand(fe),
                    pad,
                    theme::muted("tailwind"),
                    theme::brand(tw),
                    pad,
                    theme::muted("git"),
                    theme::brand(git),
                    pad,
                    theme::muted("path"),
                    theme::brand(&self.project_path().display().to_string()),
                    pad,
                    theme::key_hint(&[("enter", "create"), ("n", "back"), ("esc", "cancel")]),
                )
            }
            Step::Creating | Step::Done => {
                let job = CREATE_JOBS.get(self.create_stage).copied().unwrap_or("…");
                let status = if let Some(err) = &self.error {
                    theme::err(err)
                } else if self.create_done {
                    theme::ok("done")
                } else {
                    theme::muted(job)
                };
                format!(
                    "{}\n{}\n\n{}{}\n\n{}{}\n",
                    theme::header("Creating project"),
                    theme::muted(&format!(
                        "{} · {}",
                        self.project_name(),
                        self.project_path().display()
                    )),
                    pad,
                    self.progress.view(),
                    pad,
                    status,
                )
            }
        };

        format!(
            "\n{pad}{}\n{pad}{}\n\n{pad}{}\n\n{body}",
            theme::brand("namix"),
            theme::muted("nx new"),
            steps,
        )
    }
}

/// 运行 new 向导；name+mode+https 齐全时直接进入创建动画。
pub async fn run_new_wizard(preset: WizardPreset) -> Result<NewWizardResult, String> {
    theme::ensure_color();
    let _ = PRESET.set(preset);
    let program = Program::<NewWizard>::builder()
        .alt_screen(true)
        .signal_handler(true)
        .build()
        .map_err(|e| e.to_string())?;

    let model = program.run().await.map_err(|e| e.to_string())?;
    model.result.ok_or_else(|| "已取消".into())
}
