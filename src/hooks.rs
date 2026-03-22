use std::path::Path;

pub const PENDING_SYNC_FILE: &str = "pending-sync.json";

pub fn prepare_commit_msg_script(binary_path: &Path) -> String {
    format!(
        r#"#!/bin/sh
set -eu

SSMVER_BIN={binary_path}

if [ -x "$SSMVER_BIN" ]; then
  exec "$SSMVER_BIN" hook prepare-commit-msg "$@"
fi

if command -v ssmver >/dev/null 2>&1; then
  exec "$(command -v ssmver)" hook prepare-commit-msg "$@"
fi

echo "ssmver binary not found; rerun 'ssmver init' or install ssmver" >&2
exit 1
"#,
        binary_path = shell_quote(binary_path.as_os_str().to_string_lossy().as_ref())
    )
}

pub fn post_commit_script(binary_path: &Path) -> String {
    format!(
        r#"#!/bin/sh
set -eu

SSMVER_BIN={binary_path}

if [ -x "$SSMVER_BIN" ]; then
  exec "$SSMVER_BIN" hook post-commit
fi

if command -v ssmver >/dev/null 2>&1; then
  exec "$(command -v ssmver)" hook post-commit
fi

echo "ssmver binary not found; rerun 'ssmver init' or install ssmver" >&2
exit 1
"#,
        binary_path = shell_quote(binary_path.as_os_str().to_string_lossy().as_ref())
    )
}

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\"'\"'"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scripts_embed_shell_quoted_binary_path() {
        let script = prepare_commit_msg_script(Path::new("/tmp/it's/ssmver"));
        assert!(script.contains("'/tmp/it'\"'\"'s/ssmver'"));
    }
}
