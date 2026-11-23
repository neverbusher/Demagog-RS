use std::time::Instant;

use super::{ActiveTab, CheckReport, FindMatches, StatusLevel};

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
