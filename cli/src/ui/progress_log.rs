use crate::ui::runner::ProgressEvent;

pub fn log_started_check(event: ProgressEvent) {
    if let Some(line) = started_check_line(&event) {
        eprintln!("{line}");
    }
}

fn started_check_line(event: &ProgressEvent) -> Option<String> {
    let ProgressEvent::Started { language, name } = event else {
        return None;
    };
    let (language, workspace) = split_target_label(language);
    Some(format!(
        "running language={language} workspace={workspace} signal={name}"
    ))
}

fn split_target_label(label: &str) -> (&str, &str) {
    label.split_once(':').unwrap_or((label, "workspace"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::runner::{ProgressEvent, ToolState};
    use std::time::Duration;

    #[test]
    fn started_check_line_formats_workspace_root() {
        let event = ProgressEvent::Started {
            language: String::from("rust:workspace"),
            name: String::from("test"),
        };

        assert_eq!(
            started_check_line(&event),
            Some(String::from(
                "running language=rust workspace=workspace signal=test"
            ))
        );
    }

    #[test]
    fn started_check_line_formats_non_root_workspace() {
        let event = ProgressEvent::Started {
            language: String::from("node:apps/web"),
            name: String::from("coverage"),
        };

        assert_eq!(
            started_check_line(&event),
            Some(String::from(
                "running language=node workspace=apps/web signal=coverage"
            ))
        );
    }

    #[test]
    fn started_check_line_ignores_non_started_events() {
        let event = ProgressEvent::Finished {
            language: String::from("rust:workspace"),
            name: String::from("test"),
            state: ToolState::Done,
            elapsed: Duration::from_secs(1),
        };

        assert_eq!(started_check_line(&event), None);
    }
}
