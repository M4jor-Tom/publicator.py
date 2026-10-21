use publicator::ui::drafts::{Draft, Field};

#[test]
fn old_completion_keeps_newer_input_dirty() {
    let mut draft = Draft::new("Old".into(), "D".into());
    draft.edit(Field::Title, "First".into(), 0);
    let sent = draft.begin_save();
    draft.edit(Field::Title, "Newer".into(), 10);
    draft.saved(&sent);
    assert_eq!(draft.title(), "Newer");
    assert!(draft.is_dirty());
}

#[test]
fn title_completion_does_not_clear_newer_description() {
    let mut draft = Draft::new("Old".into(), "Old description".into());
    draft.edit(Field::Title, "New title".into(), 0);
    draft.edit(Field::Description, "First description".into(), 1);
    let sent = draft.begin_save();
    draft.edit(Field::Description, "Newer description".into(), 2);
    draft.saved(&sent);
    assert!(!draft.is_field_dirty(Field::Title));
    assert!(draft.is_field_dirty(Field::Description));
    assert_eq!(draft.description(), "Newer description");
}

#[test]
fn debounce_waits_until_three_hundred_fifty_milliseconds() {
    let mut draft = Draft::new("Title".into(), "Description".into());
    draft.edit(Field::Title, "Changed".into(), 100);
    assert!(!draft.due(449));
    assert!(draft.due(450));
}

#[test]
fn failed_save_keeps_dirty_for_explicit_retry_without_tick_spam() {
    let mut draft = Draft::new("Title".into(), "Description".into());
    draft.edit(Field::Description, "Changed".into(), 0);
    let sent = draft.begin_save();
    draft.failed(&sent);
    assert!(draft.is_dirty());
    assert!(!draft.due(10_000));
    let retry = draft.begin_save();
    draft.saved(&retry);
    assert!(!draft.is_dirty());
}

#[test]
fn discard_is_an_explicit_reset() {
    let mut draft = Draft::new("Title".into(), "Description".into());
    draft.edit(Field::Title, "Unsaved".into(), 0);
    assert!(draft.is_dirty());
    draft.discard("Title".into(), "Description".into());
    assert_eq!(draft.title(), "Title");
    assert!(!draft.is_dirty());
}
