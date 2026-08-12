//! Optional cookie / flash encryption hooks installed by [`namix::Crypt`].
//!
//! `namix-http` stays free of a hard dependency on the facade crate; Boot wires
//! encrypt/decrypt once the session secret is known.

use std::sync::OnceLock;

type SealFn = fn(&str) -> String;
type OpenFn = fn(&str) -> String;

static SEAL: OnceLock<SealFn> = OnceLock::new();
static OPEN: OnceLock<OpenFn> = OnceLock::new();

/// Install seal/open callbacks (idempotent — first install wins).
pub fn install(seal: SealFn, open: OpenFn) {
    let _ = SEAL.set(seal);
    let _ = OPEN.set(open);
}

pub(crate) fn seal_value(plaintext: &str) -> String {
    SEAL.get()
        .map(|f| f(plaintext))
        .unwrap_or_else(|| plaintext.to_string())
}

pub(crate) fn open_value(raw: &str) -> String {
    OPEN.get()
        .map(|f| f(raw))
        .unwrap_or_else(|| raw.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn passthrough_without_install() {
        assert_eq!(seal_value("ok"), "ok");
        assert_eq!(open_value("e:hi"), "e:hi");
    }
}
