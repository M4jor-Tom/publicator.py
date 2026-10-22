use std::time::Duration;

pub const DEBOUNCE_MS: u64 = 350;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum Field {
    Title,
    Description,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct FieldDraft {
    text: String,
    revision: u64,
    dirty: bool,
    last_edit_ms: Option<u64>,
    last_sent: Option<String>,
}

impl FieldDraft {
    fn new(text: String) -> Self {
        Self {
            text,
            revision: 0,
            dirty: false,
            last_edit_ms: None,
            last_sent: None,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SentField {
    pub revision: u64,
    pub text: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SentDraft {
    pub title: Option<SentField>,
    pub description: Option<SentField>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Draft {
    title: FieldDraft,
    description: FieldDraft,
    saving: bool,
}

impl Draft {
    pub fn new(title: String, description: String) -> Self {
        Self {
            title: FieldDraft::new(title),
            description: FieldDraft::new(description),
            saving: false,
        }
    }

    pub fn title(&self) -> &str {
        &self.title.text
    }

    pub fn description(&self) -> &str {
        &self.description.text
    }

    pub fn edit(&mut self, field: Field, text: String, now_ms: u64) {
        let value = self.field_mut(field);
        if value.text != text {
            value.text = text;
            value.revision += 1;
            value.dirty = true;
            value.last_edit_ms = Some(now_ms);
        }
    }

    pub fn due(&self, now_ms: u64) -> bool {
        !self.saving
            && [
                self.title.last_edit_ms.filter(|_| self.title.dirty),
                self.description
                    .last_edit_ms
                    .filter(|_| self.description.dirty),
            ]
            .into_iter()
            .flatten()
            .max()
            .is_some_and(|edited| now_ms.saturating_sub(edited) >= DEBOUNCE_MS)
    }

    pub fn begin_save(&mut self) -> SentDraft {
        self.saving = true;
        let snapshot = |field: &FieldDraft| {
            field.dirty.then(|| SentField {
                revision: field.revision,
                text: field.text.clone(),
            })
        };
        SentDraft {
            title: snapshot(&self.title),
            description: snapshot(&self.description),
        }
    }

    pub fn saved(&mut self, sent: &SentDraft) {
        Self::finish_field(&mut self.title, sent.title.as_ref());
        Self::finish_field(&mut self.description, sent.description.as_ref());
        self.saving = false;
    }

    pub fn failed(&mut self, sent: &SentDraft) {
        Self::defer_retry(&mut self.title, sent.title.as_ref());
        Self::defer_retry(&mut self.description, sent.description.as_ref());
        self.saving = false;
    }

    pub fn is_dirty(&self) -> bool {
        self.title.dirty || self.description.dirty
    }

    pub fn is_field_dirty(&self, field: Field) -> bool {
        self.field(field).dirty
    }

    pub fn is_saving(&self) -> bool {
        self.saving
    }

    pub fn last_sent(&self, field: Field) -> Option<&str> {
        self.field(field).last_sent.as_deref()
    }

    pub fn discard(&mut self, title: String, description: String) {
        *self = Self::new(title, description);
    }

    fn field(&self, field: Field) -> &FieldDraft {
        match field {
            Field::Title => &self.title,
            Field::Description => &self.description,
        }
    }

    fn field_mut(&mut self, field: Field) -> &mut FieldDraft {
        match field {
            Field::Title => &mut self.title,
            Field::Description => &mut self.description,
        }
    }

    fn defer_retry(field: &mut FieldDraft, sent: Option<&SentField>) {
        if let Some(sent) = sent
            && field.revision == sent.revision
            && field.text == sent.text
        {
            field.last_edit_ms = None;
        }
    }

    fn finish_field(field: &mut FieldDraft, sent: Option<&SentField>) {
        if let Some(sent) = sent {
            field.last_sent = Some(sent.text.clone());
            if field.revision == sent.revision && field.text == sent.text {
                field.dirty = false;
            }
        }
    }
}

pub fn debounce_interval() -> Duration {
    Duration::from_millis(DEBOUNCE_MS)
}
