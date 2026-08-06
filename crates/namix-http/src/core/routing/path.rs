#[derive(Clone, Debug)]
pub(crate) enum Segment {
    Static(String),
    Param(String),
    /// `*path`：吃掉剩余段（可为空），值为 `/` 拼接。
    Wildcard(String),
}

#[derive(Clone, Debug)]
pub(crate) struct PathPattern {
    pub raw: String,
    pub segments: Vec<Segment>,
}

impl PathPattern {
    pub fn parse(path: &str) -> Self {
        let normalized = normalize_path(path);
        let segments = if normalized == "/" {
            Vec::new()
        } else {
            normalized
                .trim_start_matches('/')
                .split('/')
                .map(|part| {
                    if let Some(name) = part.strip_prefix('*') {
                        Segment::Wildcard(name.to_string())
                    } else if let Some(name) = part.strip_prefix(':') {
                        Segment::Param(name.to_string())
                    } else {
                        Segment::Static(part.to_string())
                    }
                })
                .collect()
        };
        Self {
            raw: normalized,
            segments,
        }
    }

    pub fn join(prefix: &str, child: &str) -> String {
        let prefix = normalize_path(prefix);
        let child = child.trim();
        if child.is_empty() || child == "/" {
            return prefix;
        }
        let child = if child.starts_with('/') {
            child.to_string()
        } else {
            format!("/{child}")
        };
        if prefix == "/" {
            normalize_path(&child)
        } else {
            normalize_path(&format!("{prefix}{child}"))
        }
    }

    /// 匹配成功则按模式顺序返回参数。
    pub fn match_path_ordered(&self, path: &str) -> Option<Vec<(String, String)>> {
        let path = normalize_path(path);
        let parts: Vec<&str> = if path == "/" {
            Vec::new()
        } else {
            path.trim_start_matches('/').split('/').collect()
        };

        let mut params = Vec::new();

        if let Some(Segment::Wildcard(name)) = self.segments.last() {
            let prefix_len = self.segments.len() - 1;
            if parts.len() < prefix_len {
                return None;
            }
            for (segment, part) in self.segments.iter().take(prefix_len).zip(parts.iter()) {
                match segment {
                    Segment::Static(expected) => {
                        if expected != part {
                            return None;
                        }
                    }
                    Segment::Param(pname) => {
                        params.push((pname.clone(), (*part).to_string()));
                    }
                    Segment::Wildcard(_) => return None,
                }
            }
            params.push((name.clone(), parts[prefix_len..].join("/")));
            return Some(params);
        }

        if parts.len() != self.segments.len() {
            return None;
        }

        for (segment, part) in self.segments.iter().zip(parts.iter()) {
            match segment {
                Segment::Static(expected) => {
                    if expected != part {
                        return None;
                    }
                }
                Segment::Param(name) => {
                    params.push((name.clone(), (*part).to_string()));
                }
                Segment::Wildcard(_) => return None,
            }
        }
        Some(params)
    }
}

pub(crate) fn normalize_path(path: &str) -> String {
    if path.is_empty() {
        return "/".to_string();
    }
    let trimmed = path.trim().trim_matches('/');
    if trimmed.is_empty() {
        return "/".to_string();
    }
    let mut out = format!("/{trimmed}");
    while out.contains("//") {
        out = out.replace("//", "/");
    }
    out
}
