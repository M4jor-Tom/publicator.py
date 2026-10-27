use super::{Message, style};
use crate::prompts::{Lineage, PromptMatch};
use iced::{
    Element, Length,
    widget::{column, container, text},
};

pub fn view(match_: &PromptMatch) -> Element<'_, Message> {
    let mut lines = plain_text(match_).into_iter();
    let heading = text(lines.next().unwrap_or_default()).size(13);
    if let PromptMatch::Unknown(_) = match_ {
        return heading.color(style::MUTED).into();
    }
    let body = column(lines.map(|line| {
        container(text(line).size(13))
            .padding(6)
            .width(Length::Fill)
            .style(style::prompt_text)
            .into()
    }))
    .spacing(4);
    let block = column![heading, body].spacing(4);
    match match_ {
        PromptMatch::Exact(_) => container(block).style(style::prompt_exact),
        _ => container(block).style(style::notice),
    }
    .padding(6)
    .width(Length::Fill)
    .into()
}

pub fn plain_text(match_: &PromptMatch) -> Vec<String> {
    match match_ {
        PromptMatch::Exact(version) => {
            vec!["Archived prompt".into(), version.text.clone()]
        }
        PromptMatch::Nearest(candidates) => nearest_text(candidates),
        PromptMatch::Unknown(reason) => vec![format!("Prompt unknown: {reason}")],
    }
}

fn nearest_text(candidates: &[Lineage]) -> Vec<String> {
    let mut lines =
        vec!["Prompt NOT archived — candidate prompts only; these are not authoritative".into()];
    for lineage in candidates {
        for version in &lineage.versions {
            lines.push(format!("{} — {}", lineage.path, version.text));
        }
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::prompts::PromptVersion;

    #[test]
    fn nearest_has_uncertainty_label_and_every_candidate_lineage() {
        let match_ = PromptMatch::Nearest(vec![
            Lineage {
                path: "one/lineage.txt".into(),
                versions: vec![PromptVersion {
                    digest: "a".into(),
                    text: "first".into(),
                    paths: vec![],
                    committed: 0,
                }],
            },
            Lineage {
                path: "two/lineage.txt".into(),
                versions: vec![PromptVersion {
                    digest: "b".into(),
                    text: "second".into(),
                    paths: vec![],
                    committed: 0,
                }],
            },
        ]);
        let lines = plain_text(&match_);
        assert!(lines[0].contains("NOT archived"));
        assert!(lines[0].contains("not authoritative"));
        assert!(lines.iter().any(|line| line.contains("one/lineage.txt")));
        assert!(lines.iter().any(|line| line.contains("two/lineage.txt")));
    }
}
