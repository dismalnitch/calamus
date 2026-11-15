//! The test fonts too big for git, downloaded by `fonts/fetch.sh` (tolege#218)
//! into `fonts/fetched/`. The first font asked for runs the script, which
//! fetches whatever is missing and checks every file's SHA-256, so a fresh
//! checkout needs only a network.

use std::sync::Once;

use calamus::layout::Font;

static FETCH: Once = Once::new();

/// `fonts/fetched/<name>`, fetched first if it isn't there.
pub fn font(name: &str) -> Font {
    FETCH.call_once(|| {
        let script = concat!(env!("CARGO_MANIFEST_DIR"), "/fonts/fetch.sh");
        let out = std::process::Command::new("sh").arg(script).output().expect("couldn't run fonts/fetch.sh");
        assert!(
            out.status.success(),
            "fonts/fetch.sh failed (the first run needs a network):\n{}",
            String::from_utf8_lossy(&out.stderr)
        );
    });
    let path = format!("{}/fonts/fetched/{name}", env!("CARGO_MANIFEST_DIR"));
    let bytes = std::fs::read(&path).unwrap_or_else(|e| panic!("{path}: {e}"));
    Font::from_bytes(bytes).unwrap_or_else(|e| panic!("{path}: {e}"))
}
