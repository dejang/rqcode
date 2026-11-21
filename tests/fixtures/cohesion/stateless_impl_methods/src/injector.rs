pub struct Pasteboard {
    entries: Vec<String>,
}

struct SavedPasteboardItem {
    value: String,
}

struct ClipboardSnapshot {
    items: Vec<SavedPasteboardItem>,
}

pub struct TextInjector {
    delay_ms: u64,
}

impl TextInjector {
    pub fn new(delay_ms: u64) -> Self {
        Self { delay_ms }
    }

    pub fn try_new(delay_ms: u64) -> Option<Self> {
        Some(Self { delay_ms })
    }

    pub fn with_delay(mut self, delay_ms: u64) -> Self {
        self.delay_ms = delay_ms;
        self
    }

    pub(crate) fn with_transient_paste<T>(
        pasteboard: &mut Pasteboard,
        text: &str,
        paste_action: impl FnOnce() -> T,
    ) -> T {
        let saved = Self::snapshot_pasteboard(pasteboard);
        pasteboard.entries.push(text.to_string());
        let snapshot = ClipboardSnapshot { items: saved };
        let result = paste_action();
        Self::restore_pasteboard(pasteboard, snapshot);
        result
    }

    pub fn paste(&self, pasteboard: &mut Pasteboard, text: &str) -> u64 {
        Self::with_transient_paste(pasteboard, text, || self.delay_ms)
    }

    fn snapshot_pasteboard(pasteboard: &Pasteboard) -> Vec<SavedPasteboardItem> {
        pasteboard
            .entries
            .iter()
            .map(|e| SavedPasteboardItem { value: e.clone() })
            .collect()
    }

    fn build_pasteboard_item(saved: SavedPasteboardItem) -> String {
        saved.value
    }

    fn restore_pasteboard(pasteboard: &mut Pasteboard, snapshot: ClipboardSnapshot) {
        pasteboard.entries.clear();
        for saved in snapshot.items {
            pasteboard.entries.push(Self::build_pasteboard_item(saved));
        }
    }

    fn is_trusted() -> bool {
        true
    }
}
