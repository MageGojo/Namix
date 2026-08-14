//! `nx storage link` / `unlink` — public disk 符号链接（Laravel `storage:link`）。

use std::fs;
use std::path::{Component, Path, PathBuf};

use crate::project::Project;

pub fn link(project: &Project) -> Result<(), String> {
    let links = configured_links(project)?;
    if links.is_empty() {
        println!("没有 [storage.links] 可创建。");
        return Ok(());
    }
    for (link, target) in links {
        create_link(project, &link, &target)?;
    }
    Ok(())
}

pub fn unlink(project: &Project) -> Result<(), String> {
    let links = configured_links(project)?;
    if links.is_empty() {
        println!("没有 [storage.links] 可删除。");
        return Ok(());
    }
    for (link, _) in links {
        remove_link(project, &link)?;
    }
    Ok(())
}

fn configured_links(project: &Project) -> Result<Vec<(String, String)>, String> {
    let raw = fs::read_to_string(project.namix_toml()).unwrap_or_default();
    let value: toml::Value = toml::from_str(&raw).unwrap_or(toml::Value::Table(Default::default()));
    let Some(table) = value
        .get("storage")
        .and_then(|storage| storage.get("links"))
        .and_then(|links| links.as_table())
    else {
        return Ok(vec![("public/storage".into(), "storage/app/public".into())]);
    };
    if table.is_empty() {
        return Ok(vec![("public/storage".into(), "storage/app/public".into())]);
    }
    Ok(table
        .iter()
        .filter_map(|(link, target)| {
            target
                .as_str()
                .map(|target| (link.clone(), target.to_string()))
        })
        .collect())
}

fn create_link(project: &Project, link: &str, target: &str) -> Result<(), String> {
    let link_path = resolve(project, link);
    let target_path = resolve(project, target);
    if let Some(parent) = link_path.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("无法创建 {}: {error}", parent.display()))?;
    }
    fs::create_dir_all(&target_path)
        .map_err(|error| format!("无法创建 {}: {error}", target_path.display()))?;

    if link_path.exists() || link_path.symlink_metadata().is_ok() {
        let meta = fs::symlink_metadata(&link_path)
            .map_err(|error| format!("{}: {error}", link_path.display()))?;
        if meta.file_type().is_symlink() {
            fs::remove_file(&link_path)
                .map_err(|error| format!("无法替换 {}: {error}", link_path.display()))?;
        } else {
            return Err(format!(
                "{} 已存在且不是符号链接",
                display_path(project, &link_path)
            ));
        }
    }

    let relative = relative_from(link_path.parent().unwrap_or(&link_path), &target_path);
    symlink(&relative, &link_path)?;
    println!(
        "  {} → {}",
        display_path(project, &link_path),
        relative.display()
    );
    Ok(())
}

fn remove_link(project: &Project, link: &str) -> Result<(), String> {
    let link_path = resolve(project, link);
    match fs::symlink_metadata(&link_path) {
        Ok(meta) if meta.file_type().is_symlink() => {
            fs::remove_file(&link_path)
                .map_err(|error| format!("无法删除 {}: {error}", link_path.display()))?;
            println!("  已删除 {}", display_path(project, &link_path));
            Ok(())
        }
        Ok(_) => Err(format!(
            "{} 存在但不是符号链接，未删除",
            display_path(project, &link_path)
        )),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            println!("  不存在 {}", display_path(project, &link_path));
            Ok(())
        }
        Err(error) => Err(format!("{}: {error}", link_path.display())),
    }
}

fn resolve(project: &Project, path: &str) -> PathBuf {
    let path = Path::new(path);
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        project.app_dir.join(path)
    }
}

fn display_path(project: &Project, path: &Path) -> String {
    path.strip_prefix(&project.root)
        .unwrap_or(path)
        .display()
        .to_string()
}

fn relative_from(from_dir: &Path, to: &Path) -> PathBuf {
    let from: Vec<_> = from_dir.components().collect();
    let to: Vec<_> = to.components().collect();
    let mut i = 0;
    while i < from.len() && i < to.len() && from[i] == to[i] {
        i += 1;
    }
    let mut rel = PathBuf::new();
    for _ in i..from.len() {
        rel.push(Component::ParentDir);
    }
    for component in &to[i..] {
        rel.push(component);
    }
    if rel.as_os_str().is_empty() {
        PathBuf::from(".")
    } else {
        rel
    }
}

fn symlink(target: &Path, link: &Path) -> Result<(), String> {
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(target, link)
            .map_err(|error| format!("symlink {} → {}: {error}", link.display(), target.display()))
    }
    #[cfg(windows)]
    {
        std::os::windows::fs::symlink_dir(target, link)
            .map_err(|error| format!("symlink {} → {}: {error}", link.display(), target.display()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relative_from_walks_up_to_the_shared_prefix() {
        let rel = relative_from(Path::new("app/public"), Path::new("app/storage/app/public"));
        assert_eq!(rel, PathBuf::from("../storage/app/public"));
    }
}
