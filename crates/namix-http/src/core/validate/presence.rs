//! Laravel-style unique / exists lookups without putting SQL in Rule.

use std::sync::{Arc, RwLock};

/// Answers “does this table/column currently hold this value?”
pub trait PresenceVerifier: Send + Sync {
    fn exists(
        &self,
        table: &str,
        column: &str,
        value: &str,
        except: Option<(&str, &str)>,
    ) -> Result<bool, String>;
}

static VERIFIER: RwLock<Option<Arc<dyn PresenceVerifier>>> = RwLock::new(None);

#[cfg(test)]
pub(crate) static PRESENCE_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

pub fn install_presence_verifier(verifier: Arc<dyn PresenceVerifier>) {
    *VERIFIER.write().expect("presence verifier lock") = Some(verifier);
}

pub fn clear_presence_verifier() {
    *VERIFIER.write().expect("presence verifier lock") = None;
}

pub fn presence_exists(
    table: &str,
    column: &str,
    value: &str,
    except: Option<(&str, &str)>,
) -> Result<bool, String> {
    let guard = VERIFIER
        .read()
        .map_err(|_| "presence verifier lock poisoned".to_string())?;
    let Some(verifier) = guard.as_ref() else {
        return Err("database presence verifier is not installed".into());
    };
    verifier.exists(table, column, value, except)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    struct MapVerifier(HashMap<(String, String), Vec<(String, String)>>);

    impl PresenceVerifier for MapVerifier {
        fn exists(
            &self,
            table: &str,
            column: &str,
            value: &str,
            except: Option<(&str, &str)>,
        ) -> Result<bool, String> {
            let rows = self
                .0
                .get(&(table.to_string(), column.to_string()))
                .cloned()
                .unwrap_or_default();
            Ok(rows.iter().any(|(id, stored)| {
                stored == value && except.is_none_or(|(_, except_id)| id != except_id)
            }))
        }
    }

    #[test]
    fn reports_missing_verifier() {
        let _lock = PRESENCE_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        clear_presence_verifier();
        let err = presence_exists("users", "email", "a@b.c", None).unwrap_err();
        assert!(err.contains("not installed"));
    }

    #[test]
    fn mock_verifier_honors_except() {
        let _lock = PRESENCE_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let mut rows = HashMap::new();
        rows.insert(
            ("profiles".into(), "email".into()),
            vec![("1".into(), "a@b.c".into()), ("2".into(), "b@c.d".into())],
        );
        install_presence_verifier(Arc::new(MapVerifier(rows)));
        assert!(presence_exists("profiles", "email", "a@b.c", None).unwrap());
        assert!(!presence_exists("profiles", "email", "a@b.c", Some(("id", "1"))).unwrap());
        clear_presence_verifier();
    }
}
