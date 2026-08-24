use super::{first_notice_for_session, HookInput, Mode, Outcome};

pub(super) fn introduced_text(input: &HookInput) -> Option<String> {
    if let Some(content) = &input.tool_input.content {
        return Some(content.clone());
    }
    let mut parts: Vec<&str> = input
        .tool_input
        .new_string
        .iter()
        .map(String::as_str)
        .collect();
    parts.extend(
        input
            .tool_input
            .edits
            .iter()
            .filter_map(|edit| edit.new_string.as_deref()),
    );
    (!parts.is_empty()).then(|| parts.join("\n"))
}

pub(super) fn decide(mode: Mode, message: String) -> Outcome {
    match mode {
        Mode::Enforce => Outcome::Deny(message),
        Mode::Advise => Outcome::Notify(format!("hank (advise, not blocking): {message}")),
        Mode::Off => Outcome::Allow,
    }
}

pub(super) fn fail_open(input: &HookInput, kind: &str, reason: &str) -> Outcome {
    eprintln!("hank: policy guard failed open: {reason}");
    crate::metrics::emit("fail_open", &[("fail_kind", kind.into())]);
    if first_notice_for_session(input.session_id.as_deref(), kind) {
        return Outcome::Notify(format!(
            "hank: policy guard failed open ({reason}) — edits are UNGUARDED this session."
        ));
    }
    Outcome::Allow
}
