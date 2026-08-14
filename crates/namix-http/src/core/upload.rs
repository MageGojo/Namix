//! Multipart form parsing and uploaded-file values for FormRequest.

use std::collections::HashMap;

use bytes::Bytes;

/// One file field from `multipart/form-data`.
#[derive(Clone, Debug)]
pub struct UploadedFile {
    pub name: String,
    pub filename: String,
    pub content_type: String,
    pub data: Bytes,
}

impl UploadedFile {
    pub fn size(&self) -> usize {
        self.data.len()
    }

    pub fn is_empty(&self) -> bool {
        self.data.is_empty() && self.filename.is_empty()
    }

    pub fn extension(&self) -> &str {
        self.filename
            .rsplit('.')
            .next()
            .filter(|ext| !ext.is_empty() && *ext != self.filename)
            .unwrap_or("")
    }

    pub fn is_image(&self) -> bool {
        let mime = self.content_type.to_ascii_lowercase();
        if mime.starts_with("image/") {
            return true;
        }
        matches!(
            self.extension().to_ascii_lowercase().as_str(),
            "png" | "jpg" | "jpeg" | "gif" | "webp" | "svg"
        )
    }

    pub fn matches_mime(&self, allowed: &[&str]) -> bool {
        let ext = self.extension().to_ascii_lowercase();
        let mime = self.content_type.to_ascii_lowercase();
        allowed.iter().any(|item| {
            let item = item.trim().trim_start_matches('.').to_ascii_lowercase();
            ext == item
                || mime == item
                || mime == format!("image/{item}")
                || mime == format!("application/{item}")
                || (item == "jpg" && (ext == "jpeg" || mime == "image/jpeg"))
                || (item == "jpeg" && (ext == "jpg" || mime == "image/jpeg"))
        })
    }
}

/// Parsed `multipart/form-data` body.
#[derive(Clone, Debug, Default)]
pub struct MultipartBag {
    pub fields: HashMap<String, String>,
    pub files: HashMap<String, UploadedFile>,
}

pub fn is_multipart(content_type: &str) -> bool {
    content_type
        .to_ascii_lowercase()
        .starts_with("multipart/form-data")
}

pub fn parse_multipart(body: &[u8], content_type: &str) -> Result<MultipartBag, String> {
    let Some(boundary) = extract_boundary(content_type) else {
        return Err("multipart boundary is missing".into());
    };
    parse_multipart_body(body, &boundary)
}

fn extract_boundary(content_type: &str) -> Option<String> {
    for param in content_type.split(';').skip(1) {
        let param = param.trim();
        let Some(value) = param
            .strip_prefix("boundary=")
            .or_else(|| param.strip_prefix("BOUNDARY="))
        else {
            continue;
        };
        let value = value.trim().trim_matches('"');
        if !value.is_empty() {
            return Some(value.to_string());
        }
    }
    None
}

fn parse_multipart_body(body: &[u8], boundary: &str) -> Result<MultipartBag, String> {
    let needle = format!("--{boundary}").into_bytes();
    let mut bag = MultipartBag::default();

    let Some(first) = find_bytes(body, &needle) else {
        return Ok(bag);
    };
    let mut pos = first + needle.len();
    if body.get(pos..pos + 2) == Some(b"--") {
        return Ok(bag);
    }
    if body.get(pos..pos + 2) == Some(b"\r\n") {
        pos += 2;
    } else if body.get(pos..pos + 1) == Some(b"\n") {
        pos += 1;
    }

    while let Some(rel) = find_bytes(&body[pos..], &needle) {
        let next = pos + rel;
        let mut part = &body[pos..next];
        if part.ends_with(b"\r\n") {
            part = &part[..part.len() - 2];
        } else if part.ends_with(b"\n") {
            part = &part[..part.len() - 1];
        }
        if !part.is_empty() {
            parse_part(part, &mut bag)?;
        }
        pos = next + needle.len();
        if body.get(pos..pos + 2) == Some(b"--") {
            break;
        }
        if body.get(pos..pos + 2) == Some(b"\r\n") {
            pos += 2;
        } else if body.get(pos..pos + 1) == Some(b"\n") {
            pos += 1;
        }
    }
    Ok(bag)
}

fn parse_part(part: &[u8], bag: &mut MultipartBag) -> Result<(), String> {
    let header_end = find_bytes(part, b"\r\n\r\n")
        .map(|i| (i, 4))
        .or_else(|| find_bytes(part, b"\n\n").map(|i| (i, 2)));
    let Some((header_end, sep)) = header_end else {
        return Ok(());
    };
    let headers = std::str::from_utf8(&part[..header_end]).unwrap_or("");
    let data = &part[header_end + sep..];

    let mut name = String::new();
    let mut filename = None;
    let mut content_type = "application/octet-stream".to_string();
    for line in headers.split(['\r', '\n']).filter(|line| !line.is_empty()) {
        let (key, value) = match line.split_once(':') {
            Some((k, v)) => (k.trim(), v.trim()),
            None => continue,
        };
        if key.eq_ignore_ascii_case("content-disposition") {
            for param in value.split(';').skip(1) {
                let param = param.trim();
                if let Some(v) = param.strip_prefix("name=") {
                    name = unquote(v);
                } else if let Some(v) = param.strip_prefix("filename=") {
                    filename = Some(unquote(v));
                }
            }
        } else if key.eq_ignore_ascii_case("content-type") {
            content_type = value.to_string();
        }
    }
    if name.is_empty() {
        return Ok(());
    }
    if let Some(filename) = filename {
        bag.files.insert(
            name.clone(),
            UploadedFile {
                name,
                filename,
                content_type,
                data: Bytes::copy_from_slice(data),
            },
        );
    } else {
        bag.fields
            .insert(name, String::from_utf8_lossy(data).into_owned());
    }
    Ok(())
}

fn unquote(value: &str) -> String {
    value.trim().trim_matches('"').to_string()
}

fn find_bytes(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || haystack.len() < needle.len() {
        return None;
    }
    haystack.windows(needle.len()).position(|w| w == needle)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_text_and_file_parts() {
        let body = b"------Namix\r\nContent-Disposition: form-data; name=\"email\"\r\n\r\nuser@namix.local\r\n------Namix\r\nContent-Disposition: form-data; name=\"avatar\"; filename=\"a.png\"\r\nContent-Type: image/png\r\n\r\nPNGDATA\r\n------Namix--\r\n";
        let bag = parse_multipart(body, "multipart/form-data; boundary=----Namix").unwrap();
        assert_eq!(
            bag.fields.get("email").map(String::as_str),
            Some("user@namix.local")
        );
        let file = bag.files.get("avatar").expect("avatar");
        assert_eq!(file.filename, "a.png");
        assert_eq!(file.content_type, "image/png");
        assert_eq!(&file.data[..], b"PNGDATA");
        assert!(file.is_image());
        assert!(file.matches_mime(&["png"]));
    }
}
