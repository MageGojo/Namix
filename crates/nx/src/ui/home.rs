//! `nx` 命令面板（Charm 轻量风）。

use bubbletea_rs::{Cmd, KeyMsg, Model, Msg, Program, quit};
use bubbletea_widgets::textinput;
use crossterm::event::{KeyCode, KeyModifiers};
use lipgloss_extras::lipgloss::{Style, color::TEXT_SUBTLE};

use super::theme;

#[derive(Clone, Debug)]
pub enum HomeAction {
    New { name: Option<String> },
    Help,
    Quit,
}

pub struct HomeModel {
    input: textinput::Model,
    cursor: usize,
    commands: Vec<(&'static str, &'static str)>,
    action: Option<HomeAction>,
}

impl HomeModel {
    fn style_input(input: &mut textinput::Model) {
        input.prompt = "> ".into();
        input.prompt_style = Style::new().foreground(theme::BRAND);
        input.placeholder_style = Style::new().foreground(TEXT_SUBTLE);
        input.text_style = Style::new().foreground(theme::BRAND);
        input.completion_style = Style::new().foreground(TEXT_SUBTLE);
        input.set_placeholder("new demo");
        input.set_width(40);
        input.set_char_limit(64);
        input.set_suggestions(vec![
            "new".into(),
            "new demo".into(),
            "new blog".into(),
            "help".into(),
            "quit".into(),
            "doctor".into(),
        ]);
    }

    fn filtered(&self) -> Vec<(usize, &'static str, &'static str)> {
        let q = self.input.value().trim().to_lowercase();
        self.commands
            .iter()
            .enumerate()
            .filter(|(_, (cmd, _))| q.is_empty() || cmd.contains(&q) || cmd.starts_with(&q))
            .map(|(i, (c, d))| (i, *c, *d))
            .collect()
    }
}

impl Model for HomeModel {
    fn init() -> (Self, Option<Cmd>) {
        let mut input = textinput::new();
        Self::style_input(&mut input);
        let focus = input.focus();
        (
            Self {
                input,
                cursor: 0,
                commands: vec![
                    ("new", "创建项目"),
                    ("help", "查看用法（含 make/migrate/doctor）"),
                    ("quit", "退出"),
                ],
                action: None,
            },
            Some(focus),
        )
    }

    fn update(&mut self, msg: Msg) -> Option<Cmd> {
        if let Some(key) = msg.downcast_ref::<KeyMsg>() {
            if key.key == KeyCode::Esc
                || (key.modifiers.contains(KeyModifiers::CONTROL) && key.key == KeyCode::Char('c'))
            {
                self.action = Some(HomeAction::Quit);
                return Some(quit());
            }

            let filtered = self.filtered();
            match key.key {
                KeyCode::Down => {
                    if !filtered.is_empty() {
                        self.cursor = (self.cursor + 1).min(filtered.len() - 1);
                    }
                    return None;
                }
                KeyCode::Up => {
                    if self.cursor > 0 {
                        self.cursor -= 1;
                    }
                    return None;
                }
                KeyCode::Enter => {
                    let line = self.input.value().trim().to_string();
                    let action = if line.is_empty() {
                        filtered
                            .get(self.cursor)
                            .map(|(_, cmd, _)| *cmd)
                            .unwrap_or("new")
                            .to_string()
                    } else {
                        line
                    };
                    let mut parts = action.split_whitespace();
                    let head = parts.next().unwrap_or("new");
                    self.action = Some(match head {
                        "quit" | "q" | "exit" => HomeAction::Quit,
                        "help" | "?" => HomeAction::Help,
                        "new" => HomeAction::New {
                            name: parts.next().map(|s| s.to_string()),
                        },
                        other => HomeAction::New {
                            name: Some(other.to_string()),
                        },
                    });
                    return Some(quit());
                }
                _ => {}
            }
        }

        let before = self.input.value();
        let cmd = self.input.update(msg);
        if self.input.value() != before {
            self.cursor = 0;
        }
        cmd
    }

    fn view(&self) -> String {
        let mut list = String::new();
        for (i, (_, cmd, desc)) in self.filtered().iter().enumerate() {
            if i > 0 {
                list.push('\n');
            }
            list.push_str(&theme::select_line(i == self.cursor, cmd, desc));
        }

        format!(
            "\n  {}\n  {}\n\n  {}\n\n{}\n\n  {}\n\n",
            theme::brand("namix"),
            theme::muted("framework cli"),
            self.input.view(),
            list.lines()
                .map(|l| format!("  {l}"))
                .collect::<Vec<_>>()
                .join("\n"),
            theme::help("enter · ↑↓ · tab · esc"),
        )
    }
}

pub async fn run_home() -> Result<HomeAction, String> {
    theme::ensure_color();
    let program = Program::<HomeModel>::builder()
        .alt_screen(true)
        .signal_handler(true)
        .build()
        .map_err(|e| e.to_string())?;
    let model = program.run().await.map_err(|e| e.to_string())?;
    Ok(model.action.unwrap_or(HomeAction::Quit))
}
