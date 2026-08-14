//! SQLite-backed unique/exists lookups for FormRequest rules.

use std::path::{Path, PathBuf};

use namix_http::validate::PresenceVerifier;

pub fn sqlite_path_from_url(url: &str) -> Option<PathBuf> {
    let rest = url.strip_prefix("sqlite:")?;
    let rest = rest.strip_prefix("//").unwrap_or(rest).trim();
    if rest.is_empty() || rest == ":memory:" {
        return None;
    }
    Some(PathBuf::from(rest))
}

fn is_ident(value: &str) -> bool {
    let mut chars = value.chars();
    matches!(chars.next(), Some(c) if c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

#[derive(Clone, Debug)]
pub struct SqlitePresence {
    path: PathBuf,
}

impl SqlitePresence {
    pub fn open(path: impl AsRef<Path>) -> Self {
        Self {
            path: path.as_ref().to_path_buf(),
        }
    }
}

impl PresenceVerifier for SqlitePresence {
    fn exists(
        &self,
        table: &str,
        column: &str,
        value: &str,
        except: Option<(&str, &str)>,
    ) -> Result<bool, String> {
        if !is_ident(table) || !is_ident(column) {
            return Err("invalid table or column name".into());
        }
        if let Some((except_column, _)) = except
            && !is_ident(except_column)
        {
            return Err("invalid except column name".into());
        }
        #[cfg(feature = "sqlite")]
        {
            let conn = rusqlite::Connection::open(&self.path).map_err(|e| e.to_string())?;
            let found = if let Some((except_column, except_id)) = except {
                let sql = format!(
                    "SELECT 1 FROM {table} WHERE {column} = ?1 AND CAST({except_column} AS TEXT) != ?2 LIMIT 1"
                );
                let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;
                stmt.exists(rusqlite::params![value, except_id])
                    .map_err(|e| e.to_string())?
            } else {
                let sql = format!("SELECT 1 FROM {table} WHERE {column} = ?1 LIMIT 1");
                let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;
                stmt.exists(rusqlite::params![value])
                    .map_err(|e| e.to_string())?
            };
            Ok(found)
        }
        #[cfg(not(feature = "sqlite"))]
        {
            let _ = (&self.path, value, except);
            Err("sqlite presence verifier requires the namix sqlite feature".into())
        }
    }
}

#[cfg(all(test, feature = "sqlite"))]
mod tests {
    use super::*;

    #[test]
    fn unique_lookup_sees_inserted_row() {
        let path =
            std::env::temp_dir().join(format!("namix-presence-{}.sqlite", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let conn = rusqlite::Connection::open(&path).unwrap();
        conn.execute_batch(
            "CREATE TABLE profiles (id INTEGER PRIMARY KEY, email TEXT);
             INSERT INTO profiles (id, email) VALUES (1, 'alice@namix.local');",
        )
        .unwrap();
        let verifier = SqlitePresence::open(&path);
        assert!(
            verifier
                .exists("profiles", "email", "alice@namix.local", None)
                .unwrap()
        );
        assert!(
            !verifier
                .exists("profiles", "email", "alice@namix.local", Some(("id", "1")))
                .unwrap()
        );
        assert!(
            !verifier
                .exists("profiles", "email", "bob@namix.local", None)
                .unwrap()
        );
        let _ = std::fs::remove_file(path);
    }
}
