//! Charm 风轻量主题（参考 bubbletea progress-animated）。

use std::io::IsTerminal;

use lipgloss_extras::lipgloss::{
    ColorProfileKind, Style,
    color::{AdaptiveColor, STATUS_ERROR, STATUS_SUCCESS, TEXT_HEADER, TEXT_MUTED, TEXT_SUBTLE},
    renderer,
};

/// 品牌青绿（进度条渐变也会用到）
pub const BRAND: AdaptiveColor = AdaptiveColor {
    Light: "#0F766E",
    Dark: "#2DD4BF",
};

pub const GRADIENT_A: &str = "#14B8A6";
pub const GRADIENT_B: &str = "#38BDF8";

pub fn ensure_color() {
    if std::io::stdout().is_terminal() {
        renderer::set_color_profile(ColorProfileKind::TrueColor);
    }
}

pub fn muted(s: &str) -> String {
    Style::new().foreground(TEXT_MUTED).render(s)
}

pub fn subtle(s: &str) -> String {
    Style::new().foreground(TEXT_SUBTLE).render(s)
}

pub fn header(s: &str) -> String {
    Style::new().bold(true).foreground(TEXT_HEADER).render(s)
}

pub fn brand(s: &str) -> String {
    Style::new().bold(true).foreground(BRAND).render(s)
}

pub fn ok(s: &str) -> String {
    Style::new().foreground(STATUS_SUCCESS).render(s)
}

pub fn err(s: &str) -> String {
    Style::new().foreground(STATUS_ERROR).render(s)
}

pub fn help(s: &str) -> String {
    Style::new().foreground(TEXT_SUBTLE).render(s)
}

pub fn key_hint(keys: &[(&str, &str)]) -> String {
    keys.iter()
        .map(|(k, v)| format!("{} {}", brand(k), subtle(v)))
        .collect::<Vec<_>>()
        .join(&subtle("  ·  "))
}

/// 轻量步骤指示：`name ──●── mode ──── confirm`
pub fn step_dots(current: usize, labels: &[&str]) -> String {
    let mut out = String::new();
    for (i, label) in labels.iter().enumerate() {
        if i > 0 {
            out.push_str(&if i <= current {
                brand(" ── ")
            } else {
                subtle(" ── ")
            });
        }
        let piece = if i < current {
            ok(&format!("✓ {label}"))
        } else if i == current {
            brand(&format!("● {label}"))
        } else {
            subtle(&format!("○ {label}"))
        };
        out.push_str(&piece);
    }
    out
}

pub fn select_line(selected: bool, title: &str, desc: &str) -> String {
    if selected {
        format!("{} {}\n  {}", brand("❯"), brand(title), muted(desc))
    } else {
        format!(
            "  {}\n  {}",
            Style::new().foreground(TEXT_HEADER).render(title),
            subtle(desc)
        )
    }
}
