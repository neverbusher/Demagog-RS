use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::time::Instant;

use super::{ActiveTab, CheckReport, FindMatches, StatusLevel};
use crate::domain::{BookDocument, PipelineReport, StressFormat};

#[derive(Debug, Clone)]
pub(super) struct ResultState {
    dirty: bool,
    stale: bool,
    edited: bool,
    chunks_stale: bool,
    format: StressFormat,
}

impl ResultState {
    pub(super) fn new(format: StressFormat) -> Self {
        Self {
            dirty: false,
            stale: false,
            edited: false,
            chunks_stale: false,
            format,
        }
    }

    pub(super) fn reset(&mut self) {
        self.dirty = false;
        self.stale = false;
        self.edited = false;
        self.chunks_stale = false;
    }

    pub(super) fn mark_computed(&mut self, format: StressFormat) {
        self.dirty = true;
        self.stale = false;
        self.edited = false;
        self.chunks_stale = false;
        self.format = format;
    }

    pub(super) fn mark_reformatted(&mut self, format: StressFormat) {
        self.dirty = true;
        self.edited = false;
        self.format = format;
    }

    pub(super) fn mark_edited(&mut self) {
        self.dirty = true;
        self.edited = true;
        self.chunks_stale = true;
    }

    pub(super) fn mark_saved(&mut self) {
        self.dirty = false;
    }

    pub(super) fn mark_exported(&mut self) {
        if !self.edited {
            self.dirty = false;
        }
        self.chunks_stale = false;
    }

    pub(super) fn mark_rechunked(&mut self) {
        self.chunks_stale = false;
    }

    pub(super) fn mark_stale(&mut self) {
        self.stale = true;
    }

    pub(super) fn is_dirty(&self) -> bool {
        self.dirty
    }

    pub(super) fn is_stale(&self) -> bool {
        self.stale
    }

    pub(super) fn is_edited(&self) -> bool {
        self.edited
    }

    pub(super) fn is_chunks_stale(&self) -> bool {
        self.chunks_stale
    }

    pub(super) fn format(&self) -> StressFormat {
        self.format
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct ProcessingSnapshot {
    source_hash: u64,
}

impl ProcessingSnapshot {
    pub(super) fn capture(text: &str) -> Self {
        Self {
            source_hash: text_fingerprint(text),
        }
    }

    pub(super) fn source_changed(&self, text: &str) -> bool {
        self.source_hash != text_fingerprint(text)
    }
}

fn text_fingerprint(text: &str) -> u64 {
    let mut hasher = DefaultHasher::new();
    text.hash(&mut hasher);
    hasher.finish()
}

pub(super) struct EditorState {
    pub(super) document: BookDocument,
    pub(super) raw_pipeline_output: String,
    pub(super) source_dirty: bool,
    pub(super) result_state: ResultState,
    pub(super) chunk_apply_pending: bool,
    pub(super) raw_char_count: usize,
    pub(super) processed_char_count: usize,
}

impl EditorState {
    pub(super) fn new(document: BookDocument, format: StressFormat) -> Self {
        let raw_char_count = document.raw_text.chars().count();
        let processed_char_count = document.processed_text.chars().count();
        Self {
            document,
            raw_pipeline_output: String::new(),
            source_dirty: false,
            result_state: ResultState::new(format),
            chunk_apply_pending: false,
            raw_char_count,
            processed_char_count,
        }
    }

    pub(super) fn reset_document(&mut self, document: BookDocument) {
        self.document = document;
        self.raw_char_count = self.document.raw_text.chars().count();
        self.processed_char_count = 0;
        self.raw_pipeline_output.clear();
        self.source_dirty = false;
        self.chunk_apply_pending = false;
        self.result_state.reset();
    }
}

#[derive(Default)]
pub(super) struct ProcessingState {
    pub(super) report: PipelineReport,
    pub(super) processing_snapshot: Option<ProcessingSnapshot>,
    pub(super) capture_diff_on_run: bool,
}

pub(super) struct ViewState {
    pub(super) active_tab: ActiveTab,
    pub(super) status_line: String,
    pub(super) status_level: StatusLevel,
    pub(super) last_error: Option<String>,
    pub(super) check_report: Option<CheckReport>,
    pub(super) show_check_window: bool,
    pub(super) last_copied_chunk: Option<usize>,
    pub(super) dic_search_filter: String,
    pub(super) focus_find: bool,
    pub(super) last_window_title: String,
    pub(super) window_icon_applied: bool,
}

impl Default for ViewState {
    fn default() -> Self {
        Self {
            active_tab: ActiveTab::default(),
            status_line: String::new(),
            status_level: StatusLevel::default(),
            last_error: None,
            check_report: None,
            show_check_window: false,
            last_copied_chunk: None,
            dic_search_filter: String::new(),
            focus_find: false,
            last_window_title: "Demagog-RS".to_string(),
            window_icon_applied: false,
        }
    }
}

impl ViewState {
    pub(super) fn reset_document_state(&mut self) {
        self.active_tab = ActiveTab::default();
        self.status_line.clear();
        self.status_level = StatusLevel::default();
        self.last_error = None;
        self.check_report = None;
        self.show_check_window = false;
        self.last_copied_chunk = None;
        self.dic_search_filter.clear();
        self.focus_find = false;
    }
}

pub(super) struct SearchState {
    pub(super) find_open: bool,
    pub(super) find_query: String,
    pub(super) find_matches: FindMatches,
    pub(super) find_for_tab: Option<ActiveTab>,
    pub(super) find_dirty: bool,
    pub(super) find_last_edit: Instant,
    pub(super) find_current: usize,
    pub(super) find_go_to: Option<usize>,
}

impl Default for SearchState {
    fn default() -> Self {
        Self {
            find_open: false,
            find_query: String::new(),
            find_matches: FindMatches::default(),
            find_for_tab: None,
            find_dirty: false,
            find_last_edit: Instant::now(),
            find_current: 0,
            find_go_to: None,
        }
    }
}

impl SearchState {
    pub(super) fn reset(&mut self) {
        self.find_open = false;
        self.find_query.clear();
        self.find_matches = FindMatches::default();
        self.find_for_tab = None;
        self.find_dirty = true;
        self.find_last_edit = Instant::now();
        self.find_current = 0;
        self.find_go_to = None;
    }
}

#[derive(Default)]
pub(super) struct DiffState {
    pub(super) diff_filter: String,
    pub(super) diff_filter_cache: String,
    pub(super) diff_visible: Vec<usize>,
    pub(super) diff_cache_valid: bool,
}

impl DiffState {
    pub(super) fn reset(&mut self) {
        self.diff_filter.clear();
        self.diff_filter_cache.clear();
        self.diff_visible.clear();
        self.diff_cache_valid = false;
    }
}
