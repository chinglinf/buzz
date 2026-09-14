use std::path::Path;

use crate::managed_agents::{
    discovery::{
        classify_runtime, codex_adapter_availability, find_command, resolve_command,
        KnownAcpRuntime,
    },
    AcpAvailabilityStatus,
};

use super::{cli_probe, Requirement};

/// Requirements for CLI-login runtimes (claude, codex).
pub(super) fn requirements(
    probe_args: &[&str],
    setup_copy: &str,
    runtime: &KnownAcpRuntime,
) -> Vec<Requirement> {
    let adapter_result = runtime
        .commands
        .iter()
        .find_map(|cmd| find_command(cmd).map(|path| (*cmd, path)));
    let underlying_cli_found = runtime
        .underlying_cli
        .map(|cli| find_command(cli).is_some())
        .unwrap_or(false);

    let (availability, _cmd, adapter_path) =
        classify_runtime(adapter_result, runtime.underlying_cli, underlying_cli_found);
    let availability = if runtime.id == "codex" && availability == AcpAvailabilityStatus::Available
    {
        adapter_path
            .as_deref()
            .map(|path| codex_adapter_availability(Path::new(path)))
            .unwrap_or(availability)
    } else {
        availability
    };

    match availability {
        AcpAvailabilityStatus::Available => {
            let Some(binary_path) = resolve_command(probe_args[0]) else {
                return vec![missing_requirement(
                    probe_args,
                    setup_copy,
                    AcpAvailabilityStatus::Available,
                )];
            };
            let augmented_path = cli_probe::augmented_path();
            match cli_probe::login_probe(&binary_path, probe_args, augmented_path.as_deref()) {
                cli_probe::ProbeOutcome::LoggedIn => vec![],
                cli_probe::ProbeOutcome::LoggedOut => {
                    // Durable fallback for claude (addresses GUI-launched env/Keychain/PATH gaps
                    // between the readiness probe and the actual ACP consumer).
                    //
                    // SECURITY / CORRECTNESS:
                    // - Only bypass when the *consumer* credential file
                    //   (~/.claude/.credentials.json, read by claude-agent-acp and the
                    //   underlying Claude at start) contains a plausible accessToken.
                    // - If expiresAt is present as a unix timestamp, require it to be
                    //   in the future (small skew allowed). Past expiresAt -> setup mode.
                    // - Malformed/absent/empty -> LoggedOut (setup nudge).
                    // - This does not weaken auth: turns will still fail visibly on
                    //   real expiry/revocation at prompt time.
                    if runtime.id == "claude" {
                        if let Ok(home) = std::env::var("HOME") {
                            let cred_path = Path::new(&home)
                                .join(".claude")
                                .join(".credentials.json");
                            if claude_credentials_file_says_ready(&cred_path) {
                                return vec![];
                            }
                        }
                    }
                    vec![missing_requirement(
                        probe_args,
                        setup_copy,
                        AcpAvailabilityStatus::Available,
                    )]
                }
                cli_probe::ProbeOutcome::ConfigInvalid { stderr_excerpt } => {
                    vec![Requirement::CliConfigInvalid {
                        probe_args: probe_args.iter().map(|value| value.to_string()).collect(),
                        setup_copy: setup_copy.to_string(),
                        diagnostic: stderr_excerpt,
                    }]
                }
            }
        }
        other => vec![missing_requirement(probe_args, setup_copy, other)],
    }
}

/// Returns true only when the file the ACP consumer will read at start
/// has a non-empty accessToken and (when present) a future expiresAt.
pub(crate) fn claude_credentials_file_says_ready(cred_path: &Path) -> bool {
    let content = match std::fs::read_to_string(cred_path) {
        Ok(c) if !c.trim().is_empty() => c,
        _ => return false,
    };
    let v: serde_json::Value = match serde_json::from_str(&content) {
        Ok(v) => v,
        Err(_) => return false,
    };
    let oauth = v.get("claudeAiOauth").or_else(|| v.get("claude_ai_oauth"));
    let oauth = match oauth.and_then(|o| o.as_object()) {
        Some(o) => o,
        None => return false,
    };
    let token = match oauth.get("accessToken").and_then(|t| t.as_str()) {
        Some(t) if !t.trim().is_empty() => t,
        _ => return false,
    };
    if let Some(exp) = oauth.get("expiresAt").and_then(|e| e.as_i64()) {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        if exp <= now - 300 {
            return false;
        }
    }
    true
}

fn missing_requirement(
    probe_args: &[&str],
    setup_copy: &str,
    availability: AcpAvailabilityStatus,
) -> Requirement {
    Requirement::CliLogin {
        probe_args: probe_args.iter().map(|value| value.to_string()).collect(),
        setup_copy: setup_copy.to_string(),
        availability,
    }
}

#[cfg(test)]
mod tests {
    use super::claude_credentials_file_says_ready;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn with_creds<F: FnOnce(&std::path::Path)>(json: &str, f: F) {
        let nanos = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        let p = std::path::PathBuf::from(format!("/tmp/claude_creds_test_{}.json", nanos));
        fs::write(&p, json).expect("write");
        f(&p);
        let _ = fs::remove_file(&p);
    }

    #[test]
    fn says_ready_for_valid_future_token() {
        let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs() as i64;
        let future = now + 3600;
        let json = format!(r#"{{"claudeAiOauth":{{"accessToken":"abc123def456","expiresAt":{}}}}}"#, future);
        with_creds(&json, |p| { assert!(claude_credentials_file_says_ready(p)); });
    }

    #[test]
    fn says_not_ready_for_expired_token() {
        let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs() as i64;
        let past = now - 3600;
        let json = format!(r#"{{"claudeAiOauth":{{"accessToken":"abc123def456","expiresAt":{}}}}}"#, past);
        with_creds(&json, |p| { assert!(!claude_credentials_file_says_ready(p)); });
    }

    #[test]
    fn says_ready_when_no_expires_at_but_token_present() {
        let json = r#"{"claudeAiOauth":{"accessToken":"tok"}}"#;
        with_creds(json, |p| { assert!(claude_credentials_file_says_ready(p)); });
    }

    #[test]
    fn says_not_ready_for_empty_token() {
        let json = r#"{"claudeAiOauth":{"accessToken":""}}"#;
        with_creds(json, |p| { assert!(!claude_credentials_file_says_ready(p)); });
    }

    #[test]
    fn says_not_ready_for_null_token() {
        let json = r#"{"claudeAiOauth":{"accessToken":null}}"#;
        with_creds(json, |p| { assert!(!claude_credentials_file_says_ready(p)); });
    }

    #[test]
    fn says_not_ready_for_malformed_json() {
        let json = r#"{"claudeAiOauth":{"accessToken":"tok""#;
        with_creds(json, |p| { assert!(!claude_credentials_file_says_ready(p)); });
    }

    #[test]
    fn says_not_ready_when_file_absent() {
        let p = std::path::Path::new("/tmp/does-not-exist-cred-xyz.json");
        let _ = fs::remove_file(p);
        assert!(!claude_credentials_file_says_ready(p));
    }

    #[test]
    fn says_not_ready_for_other_shape() {
        let json = r#"{"mcpOAuth":{}}"#;
        with_creds(json, |p| { assert!(!claude_credentials_file_says_ready(p)); });
    }
}
