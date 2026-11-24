use super::*;
use crate::domain::{DictionaryKind, ProcessOutput};
use crate::services::batch::{BatchCounts, BatchOutcome, BatchProgress};
use crate::services::book_splitter::BatchSplitProgress;

#[test]
fn text_would_be_lost_for_non_empty_text() {
    assert!(text_would_be_lost("Книга"));
}

#[test]
fn processing_snapshot_detects_source_changes() {
    let snapshot = ProcessingSnapshot::capture("старый текст");
    assert!(!snapshot.source_changed("старый текст"));
    assert!(snapshot.source_changed("старый текст!"));
}

#[test]
fn text_mutations_are_disabled_while_busy() {
    assert!(!text_mutations_enabled(true));
    assert!(text_mutations_enabled(false));
}

#[test]
fn blank_text_does_not_need_confirmation() {
    assert!(!text_would_be_lost(" \n\t"));
}

#[test]
fn accepts_supported_book_extensions_case_insensitively() {
    for name in [
        "book.txt",
        "book.FB2",
        "book.fb2.zip",
        "book.epub",
        "book.docx",
        "book.zip",
        "book.MD",
        "book.log",
    ] {
        assert!(can_read_book(Path::new(name)), "{name}");
    }
}

#[test]
fn rejects_other_files() {
    for name in ["cover.png", "book", "book.rtf"] {
        assert!(!can_read_book(Path::new(name)), "{name}");
    }
}

#[test]
fn result_state_starts_clean() {
    let state = ResultState::new(StressFormat::PlusBeforeVowel);
    assert!(!state.is_dirty());
    assert!(!state.is_stale());
    assert!(!state.is_edited());
    assert_eq!(state.format(), StressFormat::PlusBeforeVowel);
}

#[test]
fn computed_result_is_dirty_but_not_edited_or_stale() {
    let mut state = ResultState::new(StressFormat::PlusBeforeVowel);
    state.mark_stale();
    state.mark_computed(StressFormat::AcuteAccent);
    assert!(state.is_dirty());
    assert!(!state.is_stale());
    assert!(!state.is_edited());
    assert!(!state.is_chunks_stale());
    assert_eq!(state.format(), StressFormat::AcuteAccent);
}

#[test]
fn editing_marks_result_dirty_and_edited() {
    let mut state = ResultState::new(StressFormat::PlusBeforeVowel);
    state.mark_computed(StressFormat::PlusBeforeVowel);
    state.mark_saved();
    state.mark_edited();
    assert!(state.is_dirty());
    assert!(state.is_edited());
    assert!(state.is_chunks_stale());
}

#[test]
fn saving_clears_dirty_but_remembers_manual_edits() {
    let mut state = ResultState::new(StressFormat::PlusBeforeVowel);
    state.mark_computed(StressFormat::PlusBeforeVowel);
    state.mark_edited();
    state.mark_saved();
    assert!(!state.is_dirty());
    assert!(
        state.is_edited(),
        "reformatting must still warn about manual edits after saving"
    );
    assert!(
        state.is_chunks_stale(),
        "saving TXT does not rebuild the chunks"
    );
}

#[test]
fn rechunking_keeps_manual_edit_status() {
    let mut state = ResultState::new(StressFormat::PlusBeforeVowel);
    state.mark_computed(StressFormat::PlusBeforeVowel);
    state.mark_saved();
    state.mark_edited();
    state.mark_rechunked();
    assert!(state.is_dirty());
    assert!(state.is_edited());
    assert!(!state.is_chunks_stale());
}

#[test]
fn rechunking_does_not_mark_saved_text_as_unsaved() {
    let mut state = ResultState::new(StressFormat::PlusBeforeVowel);
    state.mark_computed(StressFormat::PlusBeforeVowel);
    state.mark_saved();
    state.mark_rechunked();
    assert!(!state.is_dirty());
    assert!(!state.is_chunks_stale());
}

#[test]
fn empty_text_produces_no_chunks() {
    let config = crate::domain::ChunkConfig::default();
    assert!(chunks_for_text("", &config).is_empty());
    assert!(chunks_for_text("  \n\t ", &config).is_empty());
    assert!(!chunks_for_text("Первое предложение.", &config).is_empty());
}

#[test]
fn old_chunks_of_an_empty_result_cannot_be_exported() {
    let old_chunks = vec!["старый чанк".to_string()];
    assert!(!can_export_chunks("", &old_chunks));
    assert!(!can_export_chunks("  \n\t", &old_chunks));
    assert!(!can_export_chunks("Новый текст.", &[]));
    assert!(can_export_chunks("Новый текст.", &old_chunks));
}

#[test]
fn reformatting_keeps_stale_state_and_clears_edits() {
    let mut state = ResultState::new(StressFormat::PlusBeforeVowel);
    state.mark_computed(StressFormat::PlusBeforeVowel);
    state.mark_edited();
    state.mark_stale();
    state.mark_reformatted(StressFormat::CleanYoOnly);
    assert!(state.is_dirty());
    assert!(state.is_stale());
    assert!(!state.is_edited());
    assert_eq!(state.format(), StressFormat::CleanYoOnly);
}

#[test]
fn exporting_clears_dirty_only_for_unedited_results() {
    let mut state = ResultState::new(StressFormat::PlusBeforeVowel);
    state.mark_computed(StressFormat::PlusBeforeVowel);
    state.mark_exported();
    assert!(!state.is_dirty());

    state.mark_edited();
    state.mark_rechunked();
    state.mark_exported();
    assert!(state.is_dirty());
    assert!(state.is_edited());
    assert!(!state.is_chunks_stale());
}

#[test]
fn stale_result_does_not_become_dirty_on_its_own() {
    let mut state = ResultState::new(StressFormat::PlusBeforeVowel);
    state.mark_computed(StressFormat::PlusBeforeVowel);
    state.mark_saved();
    state.mark_stale();
    assert!(state.is_stale());
    assert!(!state.is_dirty());
}

#[test]
fn reset_clears_flags_but_keeps_format() {
    let mut state = ResultState::new(StressFormat::PlusBeforeVowel);
    state.mark_computed(StressFormat::AcuteAccent);
    state.mark_edited();
    state.mark_stale();
    state.reset();
    assert!(!state.is_dirty());
    assert!(!state.is_stale());
    assert!(!state.is_edited());
    assert!(!state.is_chunks_stale());
    assert_eq!(state.format(), StressFormat::AcuteAccent);
}

#[test]
fn editor_state_reset_document_clears_transient_state() {
    let document = BookDocument {
        title: "Старая книга".to_string(),
        source_path: Some(PathBuf::from("old.fb2")),
        raw_text: "Старый".to_string(),
        processed_text: "Результат".to_string(),
        chunks: vec!["Старый чанк".to_string()],
        headings: Vec::new(),
    };
    let mut state = EditorState::new(document, StressFormat::PlusBeforeVowel);
    assert_eq!(state.raw_char_count, 6);
    assert_eq!(state.processed_char_count, 9);
    state.raw_pipeline_output = "Вывод конвейера".to_string();
    state.source_dirty = true;
    state.chunk_apply_pending = true;
    state.result_state.mark_computed(StressFormat::CleanYoOnly);
    state.result_state.mark_edited();
    state.result_state.mark_stale();

    let replacement = BookDocument {
        title: "Новая книга".to_string(),
        source_path: Some(PathBuf::from("new.fb2")),
        raw_text: "Новый\nтекст".to_string(),
        processed_text: "Новый результат".to_string(),
        chunks: Vec::new(),
        headings: Vec::new(),
    };
    let raw_char_count = replacement.raw_text.chars().count();
    state.reset_document(replacement);

    assert_eq!(state.document.title, "Новая книга");
    assert_eq!(state.document.source_path, Some(PathBuf::from("new.fb2")));
    assert_eq!(state.document.raw_text, "Новый\nтекст");
    assert_eq!(state.document.processed_text, "Новый результат");
    assert!(state.document.chunks.is_empty());
    assert!(state.document.headings.is_empty());
    assert_eq!(state.raw_char_count, raw_char_count);
    assert_eq!(state.processed_char_count, 0);
    assert!(state.raw_pipeline_output.is_empty());
    assert!(!state.source_dirty);
    assert!(!state.chunk_apply_pending);
    assert!(!state.result_state.is_dirty());
    assert!(!state.result_state.is_stale());
    assert!(!state.result_state.is_edited());
    assert!(!state.result_state.is_chunks_stale());
    assert_eq!(state.result_state.format(), StressFormat::CleanYoOnly);
}

#[test]
fn find_matches_is_case_insensitive_and_handles_yo() {
    let text = "Всё всё ВСЁ";
    let found = find_matches(text, "всё");
    assert_eq!(found.ranges.len(), 3);
    assert!(!found.truncated);
    for &(start, end) in &found.ranges {
        assert_eq!(&text[start..end].to_lowercase(), "всё");
    }
}

#[test]
fn find_treats_metacharacters_literally() {
    let text = "a+b aab";
    let found = find_matches(text, "a+b");
    assert_eq!(found.ranges, vec![(0, 3)]);
}

#[test]
fn blank_query_returns_no_matches() {
    assert!(find_matches("текст", "   ").ranges.is_empty());
}

#[test]
fn find_ignores_stress_marks_in_processed_text() {
    let plus = "з+амке";
    let acute = "за\u{0301}мке";
    let text = format!("В {plus} и {acute}");
    let found = find_matches(&text, "замке");
    assert_eq!(found.ranges.len(), 2);
    assert_eq!(&text[found.ranges[0].0..found.ranges[0].1], plus);
    assert_eq!(&text[found.ranges[1].0..found.ranges[1].1], acute);
}

#[test]
fn find_treats_yo_and_e_as_equivalent() {
    let text = "все всё";
    assert_eq!(find_matches(text, "все").ranges.len(), 2);
    assert_eq!(find_matches(text, "всё").ranges.len(), 2);
}

#[test]
fn find_does_not_match_across_letters() {
    let text = "замок з+амок";
    let found = find_matches(text, "з+амок");
    assert!(!found.ranges.is_empty());
    for &(start, end) in &found.ranges {
        assert_eq!(&text[start..end], "з+амок");
    }
}

#[test]
fn find_caps_the_number_of_highlights() {
    let text = "a".repeat(MAX_FIND_MATCHES + 10);
    let found = find_matches(&text, "a");
    assert_eq!(found.ranges.len(), MAX_FIND_MATCHES);
    assert!(found.truncated);
}

#[test]
fn layout_marks_only_matching_sections() {
    let text = "abc DEF";
    let matches = [(4, 7)];
    let job = layout_editor_text(
        text,
        FontId::monospace(14.0),
        Color32::WHITE,
        20.0,
        &matches,
        None,
        200.0,
    );
    assert_eq!(job.text, text);
    assert_eq!(job.sections.len(), 2);
    assert_eq!(job.sections[0].byte_range, 0..4);
    assert_eq!(job.sections[1].byte_range, 4..7);
    assert_eq!(job.sections[1].format.background, palette::find_highlight());
    assert_eq!(job.sections[1].format.color, Color32::BLACK);
    assert_ne!(job.sections[0].format.background, palette::find_highlight());
    assert_eq!(job.sections[1].format.line_height, Some(20.0));
}

#[test]
fn layout_keeps_whole_text_when_nothing_matches() {
    let job = layout_editor_text(
        "abc",
        FontId::monospace(14.0),
        Color32::WHITE,
        20.0,
        &[],
        None,
        200.0,
    );
    assert_eq!(job.sections.len(), 1);
    assert_eq!(job.text, "abc");
}

#[test]
fn layout_marks_current_match_differently() {
    let text = "abc abc";
    let matches = [(0, 3), (4, 7)];
    let job = layout_editor_text(
        text,
        FontId::monospace(14.0),
        Color32::WHITE,
        20.0,
        &matches,
        Some(1),
        200.0,
    );
    assert_eq!(job.sections.len(), 3);
    assert_eq!(job.sections[0].format.background, palette::find_highlight());
    assert_eq!(
        job.sections[2].format.background,
        palette::find_current_highlight()
    );
}

#[test]
fn preset_all_enables_every_dictionary() {
    assert!(preset_enables(
        DictionaryPreset::All,
        DictionaryTag::PlusStressOrthoepy,
        "95.NEW_Орфоэпия(phoneme).dic"
    ));
}

#[test]
fn homographs_preset_skips_the_95_orthoepy_dictionary() {
    assert!(!preset_enables(
        DictionaryPreset::HomographsOnly,
        DictionaryTag::PlusStressOrthoepy,
        "95.NEW_Орфоэпия(phoneme).dic"
    ));
    assert!(preset_enables(
        DictionaryPreset::HomographsOnly,
        DictionaryTag::CapToPlusConverter,
        "69_TTS_Offline+УДАРЕНИЯ@(phoneme).dic"
    ));
}

#[test]
fn numbers_and_yo_preset_keeps_only_safe_tags() {
    assert!(preset_enables(
        DictionaryPreset::NumbersAndYo,
        DictionaryTag::NumbersAndDates,
        "10_REX_числа(chisla).rex"
    ));
    assert!(preset_enables(
        DictionaryPreset::NumbersAndYo,
        DictionaryTag::YoFication,
        "15_REX_всё-все(vse_vsyo).rex"
    ));
    assert!(!preset_enables(
        DictionaryPreset::NumbersAndYo,
        DictionaryTag::PlusStressWords,
        "35_мой словарь.dic"
    ));
}

#[test]
fn clear_preset_disables_every_dictionary() {
    assert!(!preset_enables(
        DictionaryPreset::Clear,
        DictionaryTag::NumbersAndDates,
        "10_REX_числа(chisla).rex"
    ));
}

#[test]
fn window_title_uses_document_name_only_for_opened_files() {
    assert_eq!(window_title("Книга", false), "Demagog-RS");
    assert_eq!(window_title("Книга", true), "Demagog-RS — Книга");
}

#[test]
fn editor_line_height_stays_comfortable() {
    for font_size in [12.0, 16.0, 19.0, 28.0, 36.0] {
        let line_height = editor_line_height(font_size);
        assert!(
            line_height >= font_size * 1.4,
            "{line_height} is too tight for {font_size}"
        );
        assert!(
            line_height <= font_size * 1.6,
            "{line_height} is too loose for {font_size}"
        );
    }
}

#[test]
fn status_live_is_quiet_while_busy_or_info() {
    use egui::accesskit::Live;
    assert_eq!(status_live(true, StatusLevel::Success), Live::Off);
    assert_eq!(status_live(false, StatusLevel::Info), Live::Off);
    assert_eq!(status_live(false, StatusLevel::Success), Live::Polite);
    assert_eq!(status_live(false, StatusLevel::Warning), Live::Polite);
    assert_eq!(status_live(false, StatusLevel::Error), Live::Polite);
}

#[test]
fn reset_settings_keeps_folder_and_restores_defaults() {
    let mut settings = AppSettings {
        dic_folder: PathBuf::from(r"F:\custom\dic"),
        ui_font_size: 24.0,
        ..AppSettings::default()
    };
    settings.set_theme(ThemeMode::Warm);
    settings
        .disabled_dictionaries
        .push("disabled.dic".to_string());

    let reset = reset_settings_preserving_folder(&settings);

    assert_eq!(reset.dic_folder, PathBuf::from(r"F:\custom\dic"));
    assert_eq!(reset.theme(), ThemeMode::Dark);
    assert_eq!(reset.ui_font_size, 16.0);
    assert!(reset.disabled_dictionaries.is_empty());
}

fn ui_test_app(raw_text: &str, processed_text: &str, chunks: Vec<String>) -> DemagogApp {
    let document = BookDocument {
        title: "Тестовая книга".to_string(),
        source_path: Some(PathBuf::from("test.fb2")),
        raw_text: raw_text.to_string(),
        processed_text: processed_text.to_string(),
        chunks,
        headings: Vec::new(),
    };
    let mut app = DemagogApp::from_parts(AppSettings::default(), document);
    app.view.window_icon_applied = true;
    app
}

#[test]
fn diff_events_render_only_the_visible_rows() {
    use crate::domain::{DictionaryKind, ReplacementEvent, StepReport};
    use egui_kittest::Harness;
    use egui_kittest::kittest::Queryable;

    let mut app = ui_test_app("", "Результат", Vec::new());
    app.view.active_tab = ActiveTab::DiffAndStats;
    app.processing.report.events = (0..1000)
        .map(|index| ReplacementEvent {
            dictionary_name: format!("dict-{index}"),
            rule_summary: format!("rule-{index}"),
            before_snippet: format!("before-{index}"),
            after_snippet: format!("after-{index}"),
        })
        .collect();
    app.processing.report.steps = vec![StepReport {
        dictionary_name: "dict".to_string(),
        kind: DictionaryKind::DicStandard,
        rule_count: 1,
        elapsed_ms: 1,
        replacements_count: 1000,
    }];

    let mut harness = Harness::builder()
        .with_size(egui::vec2(900.0, 640.0))
        .build_ui_state(|ui, app: &mut DemagogApp| app.update_ui(ui), app);
    harness.run();

    let rendered = harness.get_all_by_label_contains("dict-").count();
    assert!(
        rendered <= 40,
        "large diff list must be virtualized instead of rendering {rendered} events"
    );
}

#[test]
fn chunks_render_only_the_visible_rows() {
    use egui_kittest::Harness;
    use egui_kittest::kittest::Queryable;

    let chunks = (0..300)
        .map(|index| format!("Чанк {index}"))
        .collect::<Vec<_>>();
    let mut app = ui_test_app("", "Результат", chunks);
    app.view.active_tab = ActiveTab::Chunks;
    app.editor
        .result_state
        .mark_computed(StressFormat::PlusBeforeVowel);

    let mut harness = Harness::builder()
        .with_size(egui::vec2(900.0, 640.0))
        .build_ui_state(|ui, app: &mut DemagogApp| app.update_ui(ui), app);
    harness.run();

    let rendered = harness.get_all_by_label_contains("Чанк #").count();
    assert!(
        rendered <= 24,
        "large chunk list must be virtualized instead of rendering {rendered} cards"
    );
}

#[test]
fn finished_worker_messages_clear_cancel_flag() {
    use std::sync::mpsc;

    let (tx, rx) = mpsc::channel();

    tx.send(WorkerMessage::BookLoaded(Ok(BookDocument {
        title: "Книга".to_string(),
        source_path: None,
        raw_text: "Текст".to_string(),
        processed_text: String::new(),
        chunks: Vec::new(),
        headings: Vec::new(),
    })))
    .unwrap();

    let mut app = ui_test_app("", "Результат", Vec::new());
    app.worker.rx = Some(rx);
    app.worker.is_busy = true;
    app.worker.cancel_flag = Some(Arc::new(AtomicBool::new(false)));
    app.poll_worker();

    assert!(app.worker.cancel_flag.is_none());
    assert!(!app.worker.is_busy);
}

#[test]
fn toolbar_controls_are_reachable_at_smallest_window() {
    use egui_kittest::Harness;
    use egui_kittest::kittest::Queryable;

    for font_size in [16.0, 20.0, 28.0] {
        let mut app = ui_test_app("Исходный текст", "Результат", Vec::new());
        app.settings.ui_font_size = font_size;
        let mut harness = Harness::builder()
            .with_size(egui::vec2(1000.0, 640.0))
            .build_ui_state(|ui, app: &mut DemagogApp| app.update_ui(ui), app);
        harness.run_steps(2);

        for label in ["Открыть книгу", "Обработать", "Ещё"] {
            let control = harness.get_by_label_contains(label);
            let rect = control.rect();
            assert!(
                rect.min.y >= 0.0 && rect.max.y <= 640.0 + 0.5,
                "{label:?} at {font_size} pt is clipped vertically: {rect:?}"
            );
        }
    }
}

#[test]
fn settings_content_reaches_the_bottom_at_smallest_window() {
    use egui_kittest::Harness;
    use egui_kittest::kittest::Queryable;

    let mut app = ui_test_app("Исходный текст", "Результат", Vec::new());
    app.view.active_tab = ActiveTab::Settings;
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1000.0, 640.0))
        .build_ui_state(|ui, app: &mut DemagogApp| app.update_ui(ui), app);
    harness.run_steps(2);

    harness.get_by_label_contains("О программе").scroll_to_me();
    harness.run_steps(3);

    let rect = harness.get_by_label_contains("О программе").rect();
    assert!(
        rect.min.y >= 0.0 && rect.max.y <= 640.0 + 0.5,
        "About must be reachable in the settings scroll area: {rect:?}"
    );
}

#[test]
fn settings_expose_theme_hotkeys_and_about() {
    use egui_kittest::Harness;
    use egui_kittest::kittest::Queryable;

    let mut app = ui_test_app("Исходный текст", "Результат", Vec::new());
    app.view.active_tab = ActiveTab::Settings;
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1280.0, 900.0))
        .build_ui_state(|ui, app: &mut DemagogApp| app.update_ui(ui), app);
    harness.run();

    let app_version = format!("Demagog-RS {APP_VERSION}");
    for label in [
        "Тёмная",
        "Светлая",
        "Тёплая",
        "Горячие клавиши",
        "О программе",
        app_version.as_str(),
    ] {
        assert!(
            harness.get_all_by_label_contains(label).count() >= 1,
            "{label}"
        );
    }
}

#[test]
fn status_bar_remains_visible_at_large_user_fonts() {
    use egui_kittest::Harness;
    use egui_kittest::kittest::Queryable;

    for font_size in [16.0, 20.0, 28.0] {
        let mut app = ui_test_app("Исходный текст", "Результат", Vec::new());
        app.settings.ui_font_size = font_size;
        app.set_status(StatusLevel::Info, "Готово: загружено 34 словаря");
        let mut harness = Harness::builder()
            .with_size(egui::vec2(1000.0, 640.0))
            .build_ui_state(|ui, app: &mut DemagogApp| app.update_ui(ui), app);
        harness.run_steps(2);

        for label in ["Активно:", "Готово:"] {
            let counter = harness.get_by_label_contains(label);
            let rect = counter.rect();
            assert!(
                rect.min.y >= 0.0 && rect.max.y <= 640.0 + 0.5,
                "{label:?} at {font_size} pt is clipped vertically: {rect:?}"
            );
        }
    }
}

#[test]
fn cancel_button_requests_a_newly_cancelable_operation() {
    use egui_kittest::Harness;
    use egui_kittest::kittest::{NodeT, Queryable};
    use std::sync::atomic::AtomicBool;

    let mut app = ui_test_app("Исходный текст", "Результат", Vec::new());
    app.worker.is_busy = true;
    let cancel = Arc::new(AtomicBool::new(false));
    app.worker.cancel_flag = Some(Arc::clone(&cancel));

    let mut harness = Harness::builder()
        .with_size(egui::vec2(1120.0, 720.0))
        .build_ui_state(|ui, app: &mut DemagogApp| app.update_ui(ui), app);
    harness.run_steps(2);

    let cancel_button = harness.get_by_label_contains("Отмена");
    assert!(!cancel_button.accesskit_node().is_disabled());
    cancel_button.click();
    harness.run_steps(2);
    assert!(cancel.load(Ordering::Relaxed));
}

#[test]
fn primary_controls_stay_inside_window_at_large_user_fonts() {
    use egui_kittest::Harness;
    use egui_kittest::kittest::Queryable;

    for font_size in [16.0, 20.0, 28.0] {
        let mut app = ui_test_app("Исходный текст", "Результат", Vec::new());
        app.settings.ui_font_size = font_size;
        let mut harness = Harness::builder()
            .with_size(egui::vec2(1000.0, 640.0))
            .build_ui_state(|ui, app: &mut DemagogApp| app.update_ui(ui), app);
        harness.run();

        for label in [
            "Исходный текст (",
            "Результат (",
            "Чанки (",
            "Журнал замен",
            "Настройки",
            "Активно:",
        ] {
            let node = harness.get_by_label_contains(label);
            let rect = node.rect();
            assert!(
                rect.min.x >= 0.0 && rect.max.x <= 1000.0 + 0.5,
                "{label:?} at {font_size} pt is clipped horizontally: {rect:?}"
            );
            assert!(
                rect.min.y >= 0.0 && rect.max.y <= 640.0 + 0.5,
                "{label:?} at {font_size} pt is clipped vertically: {rect:?}"
            );
        }
    }
}

#[test]
fn poll_worker_resets_cancel_flag_on_book_load() {
    let mut app = ui_test_app("", "", Vec::new());
    app.worker.is_busy = true;
    app.worker.cancel_flag = Some(Arc::new(AtomicBool::new(false)));
    let (tx, rx) = mpsc::channel();
    tx.send(WorkerMessage::BookLoaded(Ok(BookDocument {
        title: "Тест".to_string(),
        source_path: None,
        raw_text: "Текст".to_string(),
        processed_text: String::new(),
        chunks: Vec::new(),
        headings: Vec::new(),
    })))
    .unwrap();
    app.worker.rx = Some(rx);

    app.poll_worker();

    assert!(app.worker.cancel_flag.is_none());
    assert!(!app.worker.is_busy);
}

#[test]
fn settings_about_is_scrollable_at_min_height() {
    use egui_kittest::Harness;
    use egui_kittest::kittest::Queryable;

    let mut app = ui_test_app("", "", Vec::new());
    app.view.active_tab = ActiveTab::Settings;
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1000.0, 640.0))
        .build_ui_state(|ui, app: &mut DemagogApp| app.update_ui(ui), app);
    harness.run();

    harness.get_by_label_contains("О программе").scroll_to_me();
    harness.run();

    let about = harness.get_by_label_contains("О программе");
    let rect = about.rect();
    assert!(
        rect.min.y >= 0.0 && rect.max.y <= 640.0 + 0.5,
        "about section is not visible in the viewport: {rect:?}"
    );
}

#[test]
fn journal_shows_truncation_notice() {
    use crate::domain::{DictionaryKind, ReplacementEvent, StepReport};
    use egui_kittest::Harness;
    use egui_kittest::kittest::Queryable;

    let mut app = ui_test_app("", "Результат", Vec::new());
    app.view.active_tab = ActiveTab::DiffAndStats;
    app.processing.report.events_truncated = true;
    app.processing.report.events = (0..600)
        .map(|i| ReplacementEvent {
            dictionary_name: format!("dict-{i}"),
            rule_summary: format!("rule-{i}"),
            before_snippet: format!("before-{i}"),
            after_snippet: format!("after-{i}"),
        })
        .collect();
    app.processing.report.steps = vec![StepReport {
        dictionary_name: "dict".to_string(),
        kind: DictionaryKind::DicStandard,
        rule_count: 1,
        elapsed_ms: 1,
        replacements_count: 600,
    }];

    let mut harness = Harness::builder()
        .with_size(egui::vec2(1000.0, 640.0))
        .build_ui_state(|ui, app: &mut DemagogApp| app.update_ui(ui), app);
    harness.run_steps(2);

    let notice = harness.get_by_label_contains("обрезан");
    assert!(
        notice.rect().min.y >= 0.0 && notice.rect().max.y <= 640.0,
        "truncation notice must be visible"
    );
}

#[test]
fn journal_heading_stays_visible_with_many_steps() {
    use crate::domain::{DictionaryKind, ReplacementEvent, StepReport};
    use egui_kittest::Harness;
    use egui_kittest::kittest::Queryable;

    let mut app = ui_test_app("", "Результат", Vec::new());
    app.view.active_tab = ActiveTab::DiffAndStats;
    app.processing.report.steps = (0..40)
        .map(|i| StepReport {
            dictionary_name: format!("dict-{i}"),
            kind: DictionaryKind::DicStandard,
            rule_count: 10,
            elapsed_ms: 1,
            replacements_count: 1,
        })
        .collect();
    app.processing.report.events = (0..120)
        .map(|i| ReplacementEvent {
            dictionary_name: format!("dict-{i}"),
            rule_summary: format!("rule-{i}"),
            before_snippet: format!("before-{i}"),
            after_snippet: format!("after-{i}"),
        })
        .collect();

    let mut harness = Harness::builder()
        .with_size(egui::vec2(1000.0, 640.0))
        .build_ui_state(|ui, app: &mut DemagogApp| app.update_ui(ui), app);
    harness.run_steps(2);

    let heading = harness.get_by_label_contains("🔍 Журнал");
    let heading_rect = heading.rect();
    assert!(
        heading_rect.min.y >= 0.0
            && heading_rect.max.y <= 640.0
            && heading_rect.min.x >= 0.0
            && heading_rect.max.x <= 1000.0,
        "journal heading is clipped or out of view with many timing rows: {heading_rect:?}"
    );

    let rendered = harness.get_all_by_label_contains("before-").count();
    assert!(
        rendered <= 40,
        "large diff list must be virtualized instead of rendering {rendered} snapshots"
    );
}

#[test]
fn compact_toolbar_keeps_actions_on_one_row_at_limited_width() {
    use egui_kittest::Harness;
    use egui_kittest::kittest::Queryable;

    for font_size in [16.0, 20.0, 28.0] {
        let mut app = ui_test_app("Исходный текст", "Результат", Vec::new());
        app.settings.ui_font_size = font_size;
        let mut harness = Harness::builder()
            .with_size(egui::vec2(1000.0, 640.0))
            .build_ui_state(|ui, app: &mut DemagogApp| app.update_ui(ui), app);
        harness.run();

        let open = harness.get_by_label_contains("Открыть книгу");
        let more = harness.get_by_label_contains("Ещё");
        let open_rect = open.rect();
        let more_rect = more.rect();

        assert!(
            more_rect.max.x <= 1000.0 + 0.5,
            "overflow menu is clipped at {font_size} pt: {more_rect:?}"
        );
        assert!(
            open_rect.min.y >= 0.0
                && open_rect.max.y <= 640.0
                && open_rect.min.y < 100.0
                && more_rect.min.y >= 0.0
                && more_rect.max.y <= 640.0,
            "toolbar rows overlap or are clipped at {font_size} pt: open={open_rect:?} more={more_rect:?}"
        );
    }
}

#[test]
fn stale_chunk_banner_rebuilds_chunks_through_the_ui() {
    use egui_kittest::Harness;
    use egui_kittest::kittest::Queryable;

    let mut app = ui_test_app("", "Первое предложение.", Vec::new());
    app.view.active_tab = ActiveTab::Chunks;
    app.editor
        .result_state
        .mark_computed(StressFormat::PlusBeforeVowel);
    app.editor.result_state.mark_edited();

    let mut harness = Harness::builder()
        .with_size(egui::vec2(1280.0, 900.0))
        .build_ui_state(|ui, app: &mut DemagogApp| app.update_ui(ui), app);
    harness.run();
    harness.get_by_label_contains("Пересобрать чанки").click();
    harness.run();

    assert!(!harness.state().editor.document.chunks.is_empty());
    assert!(!harness.state().editor.result_state.is_chunks_stale());
}

#[test]
fn export_button_is_disabled_for_an_empty_result() {
    use egui_kittest::Harness;
    use egui_kittest::kittest::{NodeT, Queryable};

    let app = ui_test_app("", "", vec!["старый чанк".to_string()]);
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1280.0, 900.0))
        .build_ui_state(|ui, app: &mut DemagogApp| app.update_ui(ui), app);
    harness.run();

    let export = harness.get_by_label_contains("Экспорт чанков");
    assert!(export.accesskit_node().is_disabled());
}

#[test]
fn editors_are_disabled_while_processing() {
    use egui::accesskit::Role;
    use egui_kittest::Harness;
    use egui_kittest::kittest::{NodeT, Queryable};

    let mut app = ui_test_app("Исходный текст", "", Vec::new());
    app.worker.is_busy = true;

    let mut harness = Harness::builder()
        .with_size(egui::vec2(1280.0, 900.0))
        .build_ui_state(|ui, app: &mut DemagogApp| app.update_ui(ui), app);
    harness.run_steps(2);

    assert!(
        harness
            .get_all_by_role(Role::MultilineTextInput)
            .any(|node| node.accesskit_node().is_disabled()),
        "the active text editor must be disabled while processing"
    );

    harness.state_mut().worker.is_busy = false;
    harness.run_steps(1);
    assert!(
        harness
            .get_all_by_role(Role::MultilineTextInput)
            .any(|node| !node.accesskit_node().is_disabled()),
        "the editor must become editable again after processing"
    );
}

#[test]
fn settings_show_dictionary_check_button_and_report() {
    use crate::domain::{DictionaryPurpose, DictionaryStage};
    use crate::services::dictionary_audit::{
        AuditCandidate, AuditKind, AuditReport, DictionaryAudit,
    };
    use crate::services::dictionary_check::{
        CheckDiagnostic, CheckReport, GOLDEN_CASES, OrderDiagnostic, OrderReport, OrderedDictionary,
    };
    use egui_kittest::Harness;
    use egui_kittest::kittest::Queryable;

    let order = OrderReport {
        ordered: vec![OrderedDictionary {
            name: "10_REX_числа(chisla).rex".to_string(),
            stage: DictionaryStage::Rex,
        }],
        warnings: vec![CheckDiagnostic::Order(OrderDiagnostic::RexWithoutNumber {
            name: "numbers.rex".to_string(),
        })],
        failures: vec![CheckDiagnostic::Order(OrderDiagnostic::Unnumbered {
            name: "custom.dic".to_string(),
        })],
    };
    let audit = AuditReport {
        dictionaries: vec![DictionaryAudit {
            name: "67_TTS_Offline АВТОР(phoneme).dic".to_string(),
            purpose: DictionaryPurpose::Author,
            total_rules: 10,
            candidates: vec![AuditCandidate {
                line: 1,
                lhs: "Всьо".to_string(),
                rhs: "Всё".to_string(),
                kind: AuditKind::Letters,
            }],
        }],
        skipped: vec![(
            "65_ЛАТИНИЦА@.dic".to_string(),
            DictionaryPurpose::Transliteration,
        )],
        scanned_rules: 10,
        total_candidates: 1,
    };
    let report = CheckReport {
        order,
        audit,
        loaded: 1,
        total_rules: 10,
        passed: GOLDEN_CASES.len(),
    };

    let mut app = ui_test_app("", "", Vec::new());
    app.view.active_tab = ActiveTab::Settings;
    app.view.check_report = Some(report);
    app.view.show_check_window = true;

    let mut harness = Harness::builder()
        .with_size(egui::vec2(1280.0, 900.0))
        .build_ui_state(|ui, app: &mut DemagogApp| app.update_ui(ui), app);
    harness.run();

    harness.get_by_label_contains("Проверить словари");
    harness.get_by_label_contains("🔎 Проверка словарей");
    harness.get_by_label_contains("Фонетические кандидаты");
    harness.get_by_label_contains("Кандидатов: 1 из 10");
    harness.get_by_label_contains("1. [REX] 10_REX_числа(chisla).rex");
    harness.get_by_label_contains("• numbers.rex: .rex без номера — проверьте порядок");
    harness.get_by_label_contains(
        "• custom.dic: .dic без номера — загрузчик поставит его после HARD-E",
    );
    harness.get_by_label_contains("67_TTS_Offline АВТОР(phoneme).dic — 1 из 10 [автор]");
}

#[test]
#[ignore]
fn clicking_dictionary_check_loads_report() {
    use egui_kittest::Harness;
    use egui_kittest::kittest::Queryable;

    let mut app = ui_test_app("", "", Vec::new());
    app.view.active_tab = ActiveTab::Settings;
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1280.0, 900.0))
        .build_ui_state(|ui, app: &mut DemagogApp| app.update_ui(ui), app);
    harness.run();

    harness.get_by_label_contains("Проверить словари").click();
    harness.run_steps(1);

    // Generous deadline: under parallel test/agent load the debug dictionary
    // compile can be slow; run ignored tests serially for stable timing.
    let deadline = Instant::now() + Duration::from_secs(120);
    while harness.state().view.check_report.is_none() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(50));
        harness.run_steps(1);
    }

    assert!(
        harness.state().view.check_report.is_some(),
        "dictionary check did not finish within 60 seconds"
    );
    assert!(harness.state().view.show_check_window);
    harness.get_by_label_contains("Фонетические кандидаты");
}

struct NoopProcessor;

impl TextProcessor for NoopProcessor {
    fn name(&self) -> &str {
        "loaded.dic"
    }
    fn kind(&self) -> DictionaryKind {
        DictionaryKind::DicStandard
    }
    fn rule_count(&self) -> usize {
        0
    }
    fn process(&self, text: &str, _capture_diff: bool) -> ProcessOutput {
        ProcessOutput {
            text: text.to_string(),
            replacements_count: 0,
            events: Vec::new(),
        }
    }
}

#[test]
fn sync_keeps_disabled_names_for_unloaded_dictionaries() {
    let mut app = ui_test_app("", "", Vec::new());
    app.settings.disabled_dictionaries = vec!["missing.dic".to_string(), "loaded.dic".to_string()];
    app.dictionaries = vec![LoadedDictionary {
        meta: DictionaryMeta {
            name: "loaded.dic".to_string(),
            kind: DictionaryKind::DicStandard,
            tag: DictionaryTag::CleanupAndTranslit,
            description: String::new(),
            outputs_plus_stress: false,
            enabled: true,
            rule_count: 0,
            encoding: "UTF-8",
            warnings: Vec::new(),
            last_ms: None,
            last_replacements: None,
        },
        processor: Arc::new(NoopProcessor),
    }];

    app.sync_disabled_dictionaries();

    assert!(
        app.settings
            .disabled_dictionaries
            .contains(&"missing.dic".to_string()),
        "disabled entry for a not-loaded dictionary must survive: {:?}",
        app.settings.disabled_dictionaries
    );
    assert!(
        !app.settings
            .disabled_dictionaries
            .contains(&"loaded.dic".to_string()),
        "a loaded-and-enabled dictionary must not stay disabled: {:?}",
        app.settings.disabled_dictionaries
    );
}

#[test]
fn escape_closes_find_while_cancel_is_pending() {
    use egui_kittest::Harness;

    let mut app = ui_test_app("текст", "", Vec::new());
    app.worker.is_busy = true;
    app.worker.cancel_flag = Some(Arc::new(AtomicBool::new(true)));
    app.search.find_open = true;

    let mut harness = Harness::builder()
        .with_size(egui::vec2(1280.0, 900.0))
        .build_ui_state(|ui, app: &mut DemagogApp| app.update_ui(ui), app);
    harness.run_steps(1);
    harness.key_press(egui::Key::Escape);
    harness.run_steps(1);

    assert!(
        !harness.state().search.find_open,
        "Esc must close the find bar when a cancel is already pending"
    );
}

#[test]
fn save_and_export_are_disabled_while_busy() {
    use egui_kittest::Harness;
    use egui_kittest::kittest::{NodeT, Queryable};

    let mut app = ui_test_app("исходник", "результат", vec!["чанк".to_string()]);
    app.worker.is_busy = true;
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1280.0, 900.0))
        .build_ui_state(|ui, app: &mut DemagogApp| app.update_ui(ui), app);
    harness.run_steps(2);

    for label in ["Сохранить результат", "Экспорт чанков"] {
        let node = harness.get_by_label_contains(label);
        assert!(
            node.accesskit_node().is_disabled(),
            "{label} must be disabled while busy"
        );
    }
}

#[test]
fn chunk_height_estimate_uses_actual_longest_chunk() {
    let chunks = vec!["короткий".to_string(), "ещё".to_string()];
    assert_eq!(estimated_chunk_lines(&chunks, 600, 10.0), 1.0);
    assert_eq!(estimated_chunk_lines(&[], 600, 10.0), 60.0);
}

#[test]
fn font_settings_save_waits_for_drag_end() {
    assert!(!should_save_font_settings(true, true, false, false));
    assert!(should_save_font_settings(false, false, true, false));
    assert!(should_save_font_settings(true, false, false, false));
    assert!(should_save_font_settings(false, false, false, true));
}

#[test]
fn find_counter_clamps_stale_current_match() {
    use egui_kittest::Harness;
    use egui_kittest::kittest::{NodeT, Queryable};

    let mut app = ui_test_app("текст текст", "", Vec::new());
    app.search.find_open = true;
    app.search.find_query = "тек".to_string();
    app.search.find_for_tab = Some(app.view.active_tab);
    // Не dirty: debounce не должен сбрасывать find_current до рендера счётчика.
    app.search.find_dirty = false;
    app.search.find_matches = FindMatches {
        ranges: vec![(0, 3), (6, 9)],
        truncated: false,
    };
    app.search.find_current = 5;

    let mut harness = Harness::builder()
        .with_size(egui::vec2(1280.0, 900.0))
        .build_ui_state(|ui, app: &mut DemagogApp| app.update_ui(ui), app);
    harness.run_steps(1);

    let counter = harness.get_by_label_contains("из 2");
    let label = counter.accesskit_node().value().unwrap_or_default();
    assert!(
        label.contains("2 из 2"),
        "stale counter must clamp: {label}"
    );
}

#[test]
fn save_source_is_disabled_while_busy() {
    use egui_kittest::Harness;
    use egui_kittest::kittest::{NodeT, Queryable};

    let mut app = ui_test_app("исходник", "", Vec::new());
    app.worker.is_busy = true;
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1280.0, 900.0))
        .build_ui_state(|ui, app: &mut DemagogApp| app.update_ui(ui), app);
    harness.run_steps(2);

    let node = harness.get_by_label_contains("Сохранить исходный текст");
    assert!(
        node.accesskit_node().is_disabled(),
        "save source must be disabled while busy"
    );
}

#[test]
fn replacing_source_with_result_marks_result_stale() {
    use egui_kittest::Harness;
    use egui_kittest::kittest::Queryable;

    let mut app = ui_test_app("", "новый результат", Vec::new());
    app.view.active_tab = ActiveTab::ProcessedText;
    app.editor
        .result_state
        .mark_computed(StressFormat::PlusBeforeVowel);

    let mut harness = Harness::builder()
        .with_size(egui::vec2(1280.0, 900.0))
        .build_ui_state(|ui, app: &mut DemagogApp| app.update_ui(ui), app);
    harness.run();
    harness
        .get_by_label_contains("Заменить исходный текст")
        .click();
    harness.run_steps(1);

    assert_eq!(harness.state().editor.document.raw_text, "новый результат");
    assert!(
        harness.state().editor.result_state.is_stale(),
        "result derived from the old source must be marked stale after replace"
    );
}

#[test]
fn settings_cleanup_checkboxes_toggle_and_mark_result_stale() {
    use egui_kittest::Harness;
    use egui_kittest::kittest::Queryable;

    let mut app = ui_test_app("Глава первая", "Глава первая", vec!["Глава первая".into()]);
    app.view.active_tab = ActiveTab::Settings;
    app.editor
        .result_state
        .mark_computed(StressFormat::PlusBeforeVowel);

    let mut harness = Harness::builder()
        .with_size(egui::vec2(1280.0, 900.0))
        .build_ui_state(|ui, app: &mut DemagogApp| app.update_ui(ui), app);
    harness.run();

    harness
        .get_by_label_contains("Удалять пустые строки")
        .click();
    harness.run();
    assert!(harness.state().settings.cleanup.remove_empty_lines);
    assert!(harness.state().editor.result_state.is_stale());

    harness
        .get_by_label_contains("Ставить точку в конце строки")
        .click();
    harness.run();
    assert!(harness.state().settings.cleanup.add_line_end_periods);
}

#[test]
fn diff_tab_shows_cleanup_stats_when_normalization_changed_text() {
    use egui_kittest::Harness;
    use egui_kittest::kittest::Queryable;

    let mut app = ui_test_app("Глава", "Глава.", vec!["Глава.".into()]);
    app.view.active_tab = ActiveTab::DiffAndStats;
    app.processing.report.cleanup = crate::domain::NormalizationStats {
        removed_empty_lines: 2,
        added_periods: 3,
    };

    let mut harness = Harness::builder()
        .with_size(egui::vec2(1280.0, 900.0))
        .build_ui_state(|ui, app: &mut DemagogApp| app.update_ui(ui), app);
    harness.run();

    assert!(
        harness
            .get_all_by_label_contains("-2 строк, +3 точек")
            .count()
            >= 1
    );
}

#[test]
fn batch_queue_filters_dedups_and_defaults_output_dir() {
    let dir = std::env::temp_dir();
    let a = dir.join("demagog-batch-ui-a.txt");
    let b = dir.join("demagog-batch-ui-b.fb2");
    let png = dir.join("demagog-batch-ui-c.png");

    let mut state = batch::BatchState::default();
    state.add_files([a.clone(), a, b, png]);

    assert_eq!(state.items.len(), 2);
    assert_eq!(state.output_dir, Some(dir.join("processed")));

    state.clear();
    assert!(state.items.is_empty());
    assert!(state.output_dir.is_none());
}

#[test]
fn batch_can_start_requires_files_format_and_output_dir() {
    let mut state = batch::BatchState::default();
    assert!(!state.can_start(false));

    state.add_files([std::env::temp_dir().join("demagog-batch-ui-book.txt")]);
    assert!(state.can_start(false));
    assert!(!state.can_start(true));

    state.export_txt = false;
    state.export_chunks = false;
    assert!(!state.can_start(false));

    state.export_chunks = true;
    state.output_dir = None;
    assert!(!state.can_start(false));
}

#[test]
fn batch_window_opens_from_toolbar_with_disabled_start_for_empty_queue() {
    use egui_kittest::Harness;
    use egui_kittest::kittest::{NodeT, Queryable};

    let app = ui_test_app("Текст", "", Vec::new());
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1280.0, 900.0))
        .build_ui_state(|ui, app: &mut DemagogApp| app.update_ui(ui), app);
    harness.run();

    harness.get_by_label_contains("📚 Пакет").click();
    harness.run();

    assert!(harness.state().batch.open);
    assert!(
        harness
            .get_all_by_label_contains("Папка результатов")
            .count()
            >= 1
    );
    let start = harness.get_by_label_contains("Старт");
    assert!(start.accesskit_node().is_disabled());
}

#[test]
fn batch_window_shows_queued_file_and_enables_start() {
    use egui_kittest::Harness;
    use egui_kittest::kittest::{NodeT, Queryable};

    let mut app = ui_test_app("Текст", "", Vec::new());
    app.batch.open = true;
    app.batch
        .add_files([std::env::temp_dir().join("demagog-batch-ui-book.txt")]);

    let mut harness = Harness::builder()
        .with_size(egui::vec2(1280.0, 900.0))
        .build_ui_state(|ui, app: &mut DemagogApp| app.update_ui(ui), app);
    harness.run();

    assert!(
        harness
            .get_all_by_label_contains("demagog-batch-ui-book.txt")
            .count()
            >= 1
    );
    let start = harness.get_by_label_contains("Старт");
    assert!(!start.accesskit_node().is_disabled());
}

#[test]
fn batch_cancel_resets_running_items_to_queued() {
    use std::sync::mpsc;

    let mut app = ui_test_app("", "", Vec::new());
    app.batch.add_files([
        std::env::temp_dir().join("demagog-batch-cancel-a.txt"),
        std::env::temp_dir().join("demagog-batch-cancel-b.txt"),
    ]);
    app.batch.running = true;
    app.worker.is_busy = true;

    let (tx, rx) = mpsc::channel();
    app.worker.rx = Some(rx);
    tx.send(WorkerMessage::Batch(BatchProgress::FileStarted {
        index: 0,
        total: 2,
        name: "demagog-batch-cancel-a.txt".to_string(),
    }))
    .unwrap();
    let outcome = BatchOutcome::Cancelled(BatchCounts {
        succeeded: 1,
        failed: 2,
    });
    tx.send(WorkerMessage::Batch(BatchProgress::Finished(outcome)))
        .unwrap();
    drop(tx);

    app.poll_worker();

    assert!(matches!(
        app.batch.items[0].status,
        batch::BatchItemStatus::Queued
    ));
    assert!(!app.batch.running);
    assert!(!app.worker.is_busy);
    assert_eq!(app.batch.summary, Some(outcome));
    assert_eq!(app.worker.progress_ratio, 0.0);
    assert_eq!(app.view.status_line, "Пакетная обработка отменена");
    assert_eq!(app.view.status_level, StatusLevel::Warning);
}

#[test]
fn batch_completion_uses_service_counts_and_finishes_progress() {
    use std::sync::mpsc;

    let mut app = ui_test_app("", "", Vec::new());
    app.batch.running = true;
    app.worker.is_busy = true;
    let outcome = BatchOutcome::Completed(BatchCounts {
        succeeded: 3,
        failed: 2,
    });

    let (tx, rx) = mpsc::channel();
    app.worker.rx = Some(rx);
    tx.send(WorkerMessage::Batch(BatchProgress::Finished(outcome)))
        .unwrap();
    drop(tx);

    app.poll_worker();

    assert!(!app.batch.running);
    assert!(!app.worker.is_busy);
    assert_eq!(app.batch.summary, Some(outcome));
    assert_eq!(app.worker.progress_ratio, 1.0);
    assert_eq!(app.view.status_line, "Пакет готов: обработано 3, ошибок 2");
    assert_eq!(app.view.status_level, StatusLevel::Warning);
}

#[test]
fn batch_run_through_app_worker_writes_outputs_and_updates_queue() {
    use crate::domain::TextCleanupConfig;

    let root = std::env::temp_dir().join(format!(
        "demagog-batch-e2e-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let sub = root.join("sub");
    std::fs::create_dir_all(&sub).unwrap();
    let book = sub.join("alpha.txt");
    std::fs::write(&book, "Глава первая\n\nТекст.").unwrap();
    let out = root.join("out");

    let mut app = ui_test_app("", "", Vec::new());
    app.batch.open = true;
    app.batch.add_folder_files([book], &root);
    app.batch.output_dir = Some(out.clone());
    app.settings.cleanup = TextCleanupConfig {
        remove_empty_lines: true,
        add_line_end_periods: true,
    };

    app.start_batch(egui::Context::default());
    assert!(app.worker.is_busy);
    assert!(app.batch.running);

    let deadline = Instant::now() + Duration::from_secs(15);
    while app.worker.is_busy && Instant::now() < deadline {
        app.poll_worker();
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(
        !app.worker.is_busy,
        "batch did not finish within the deadline"
    );

    assert!(!app.batch.running);
    assert!(matches!(
        app.batch.items[0].status,
        batch::BatchItemStatus::Done(_)
    ));
    let outcome = app.batch.summary.expect("summary set");
    assert_eq!(
        outcome,
        BatchOutcome::Completed(BatchCounts {
            succeeded: 1,
            failed: 0,
        })
    );
    assert_eq!(app.worker.progress_ratio, 1.0);
    assert_eq!(app.view.status_line, "Пакет готов: обработано 1, ошибок 0");
    assert_eq!(app.view.status_level, StatusLevel::Success);
    assert_eq!(
        std::fs::read_to_string(out.join("sub").join("alpha.txt")).unwrap(),
        "Глава первая.\nТекст."
    );
    assert!(out.join("sub").join("alpha_chunks.json").exists());

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn batch_folder_add_keeps_base_dir_and_individual_add_uses_parent() {
    let dir = std::env::temp_dir();
    let book = dir.join("demagog-batch-base-book.txt");

    let mut state = batch::BatchState::default();
    state.add_folder_files([book.clone()], &dir);
    assert_eq!(state.items[0].base_dir, dir);
    assert_eq!(state.output_dir, Some(dir.join("processed")));

    state.clear();
    state.add_files([book.clone()]);
    assert_eq!(state.items[0].base_dir, dir);
}

#[test]
fn batch_restore_drops_missing_files() {
    let dir = std::env::temp_dir().join(format!(
        "demagog-batch-restore-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let existing = dir.join("here.txt");
    std::fs::write(&existing, "текст").unwrap();

    let entries = vec![
        crate::domain::BatchQueueEntry {
            path: existing.clone(),
            base_dir: dir.clone(),
        },
        crate::domain::BatchQueueEntry {
            path: dir.join("gone.txt"),
            base_dir: dir.clone(),
        },
    ];
    let (items, dropped) = batch::restore_queue(&entries);

    assert_eq!(items.len(), 1);
    assert_eq!(items[0].path, existing);
    assert_eq!(items[0].base_dir, dir);
    assert_eq!(dropped, 1);

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn batch_scan_excludes_output_subtree() {
    let dir = std::env::temp_dir().join(format!(
        "demagog-batch-scan-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let processed = dir.join("processed");
    let deep = processed.join("sub");
    std::fs::create_dir_all(&deep).unwrap();
    std::fs::write(dir.join("book.txt"), "книга").unwrap();
    std::fs::write(processed.join("out.txt"), "результат").unwrap();
    std::fs::write(deep.join("deep.txt"), "результат").unwrap();

    let scanned = batch::collect_books_recursive(&dir, Some(&processed));
    assert_eq!(scanned, vec![dir.join("book.txt")]);

    let all = batch::collect_books_recursive(&dir, None);
    assert_eq!(all.len(), 3);

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn batch_state_syncs_to_settings_and_restores() {
    let dir = std::env::temp_dir().join(format!(
        "demagog-batch-sync-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let sub = dir.join("sub");
    std::fs::create_dir_all(&sub).unwrap();
    let book = sub.join("book.txt");
    std::fs::write(&book, "текст").unwrap();

    let mut app = ui_test_app("", "", Vec::new());
    app.batch.add_folder_files([book], &dir);
    app.batch.export_txt = false;
    app.sync_batch_settings();

    assert_eq!(app.settings.batch.queue.len(), 1);
    assert_eq!(app.settings.batch.queue[0].base_dir, dir);
    assert!(!app.settings.batch.export_txt);
    assert_eq!(app.settings.batch.output_dir, Some(dir.join("processed")));

    let document = BookDocument {
        title: "Тест".to_string(),
        source_path: None,
        raw_text: String::new(),
        processed_text: String::new(),
        chunks: Vec::new(),
        headings: Vec::new(),
    };
    let restored = DemagogApp::from_parts(app.settings.clone(), document);
    assert_eq!(restored.batch.items.len(), 1);
    assert_eq!(restored.batch.items[0].base_dir, dir);
    assert!(!restored.batch.export_txt);

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn knigorez_tab_previews_parts_on_open() {
    use egui_kittest::Harness;
    use egui_kittest::kittest::Queryable;

    let mut app = ui_test_app(
        "Вступление.\nГлава 1\nТекст один.\nГлава 2\nТекст два.",
        "",
        Vec::new(),
    );
    app.view.active_tab = ActiveTab::BookCutter;
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1100.0, 800.0))
        .build_ui_state(|ui, app: &mut DemagogApp| app.update_ui(ui), app);
    harness.run();

    assert_eq!(harness.get_all_by_label_contains("Частей: 3").count(), 1);
    assert!(harness.get_all_by_label_contains("01 Вступление").count() >= 1);
    harness.get_by_label("🔍 Тест").click();
    harness.run();
    assert_eq!(harness.get_all_by_label_contains("Частей: 3").count(), 1);
}

#[test]
fn knigorez_empty_keywords_show_validation_error() {
    use egui_kittest::Harness;
    use egui_kittest::kittest::Queryable;

    let mut app = ui_test_app("Глава 1\nТекст.", "", Vec::new());
    app.settings.split.keywords = "  ".to_string();
    app.view.active_tab = ActiveTab::BookCutter;
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1100.0, 800.0))
        .build_ui_state(|ui, app: &mut DemagogApp| app.update_ui(ui), app);
    harness.run();

    assert!(harness.get_all_by_label_contains("ключевых слов").count() >= 1);
}

#[test]
fn knigorez_tab_has_batch_queue_section() {
    use egui_kittest::Harness;
    use egui_kittest::kittest::{NodeT, Queryable};

    let mut app = ui_test_app("Глава 1\nТекст.", "", Vec::new());
    app.editor.document.source_path = None;
    app.view.active_tab = ActiveTab::BookCutter;
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1100.0, 800.0))
        .build_ui_state(|ui, app: &mut DemagogApp| app.update_ui(ui), app);
    harness.run();

    assert_eq!(
        harness
            .get_all_by_label_contains("Пакетная нарезка")
            .count(),
        1
    );
    assert_eq!(
        harness.get_all_by_label_contains("Добавить файлы").count(),
        1
    );
    assert!(
        harness
            .get_by_label("▶ Нарезать пакет")
            .accesskit_node()
            .is_disabled(),
        "start must be disabled with no files"
    );
}

#[test]
fn knigorez_batch_cancellation_resets_running_and_renders_service_counts() {
    use egui_kittest::Harness;
    use egui_kittest::kittest::Queryable;
    use std::sync::mpsc;

    let mut app = ui_test_app("", "", Vec::new());
    app.editor.document.source_path = None;
    app.view.active_tab = ActiveTab::BookCutter;
    app.splitter.batch_queue.add_files([
        std::env::temp_dir().join("demagog-knigorez-cancel-a.txt"),
        std::env::temp_dir().join("demagog-knigorez-cancel-b.txt"),
    ]);
    app.splitter.batch_queue.running = true;
    app.worker.is_busy = true;
    app.worker.cancel_flag = Some(Arc::new(AtomicBool::new(false)));

    let (tx, rx) = mpsc::channel();
    app.worker.rx = Some(rx);
    tx.send(WorkerMessage::BookSplit(BatchSplitProgress::FileStarted {
        index: 0,
        total: 2,
        name: "demagog-knigorez-cancel-a.txt".to_string(),
    }))
    .unwrap();
    tx.send(WorkerMessage::BookSplit(BatchSplitProgress::FileFinished {
        index: 0,
        outcome: Ok(2),
    }))
    .unwrap();
    tx.send(WorkerMessage::BookSplit(BatchSplitProgress::FileStarted {
        index: 1,
        total: 2,
        name: "demagog-knigorez-cancel-b.txt".to_string(),
    }))
    .unwrap();
    let outcome = BatchOutcome::Cancelled(BatchCounts {
        succeeded: 1,
        failed: 0,
    });
    tx.send(WorkerMessage::BookSplit(BatchSplitProgress::Finished(
        outcome,
    )))
    .unwrap();
    drop(tx);

    app.poll_worker();

    assert!(matches!(
        app.splitter.batch_queue.statuses[0],
        splitter::BatchSplitStatus::Done(2)
    ));
    assert!(matches!(
        app.splitter.batch_queue.statuses[1],
        splitter::BatchSplitStatus::Queued
    ));
    assert!(!app.splitter.batch_queue.running);
    assert!(!app.worker.is_busy);
    assert!(app.worker.cancel_flag.is_none());
    assert_eq!(app.splitter.batch_queue.summary, Some(outcome));
    assert_eq!(app.worker.progress_ratio, 1.0);
    assert_eq!(app.view.status_line, "Отменено: нарезано 1, ошибок 0");
    assert_eq!(app.view.status_level, StatusLevel::Success);
    app.view.status_line.clear();

    let mut harness = Harness::builder()
        .with_size(egui::vec2(1100.0, 800.0))
        .build_ui_state(|ui, app: &mut DemagogApp| app.update_ui(ui), app);
    harness.run();
    assert_eq!(
        harness
            .get_all_by_label("Отменено: нарезано 1, ошибок 0")
            .count(),
        1
    );
}

#[test]
fn knigorez_batch_completion_uses_service_counts_and_renders_summary() {
    use egui_kittest::Harness;
    use egui_kittest::kittest::Queryable;
    use std::sync::mpsc;

    let mut app = ui_test_app("", "", Vec::new());
    app.editor.document.source_path = None;
    app.view.active_tab = ActiveTab::BookCutter;
    app.splitter.batch_queue.add_files([
        std::env::temp_dir().join("demagog-knigorez-done-a.txt"),
        std::env::temp_dir().join("demagog-knigorez-done-b.txt"),
    ]);
    app.splitter.batch_queue.running = true;
    app.worker.is_busy = true;
    app.worker.cancel_flag = Some(Arc::new(AtomicBool::new(false)));

    let (tx, rx) = mpsc::channel();
    app.worker.rx = Some(rx);
    tx.send(WorkerMessage::BookSplit(BatchSplitProgress::FileFinished {
        index: 0,
        outcome: Ok(2),
    }))
    .unwrap();
    tx.send(WorkerMessage::BookSplit(BatchSplitProgress::FileFinished {
        index: 1,
        outcome: Err("ошибка".to_string()),
    }))
    .unwrap();
    let outcome = BatchOutcome::Completed(BatchCounts {
        succeeded: 3,
        failed: 2,
    });
    tx.send(WorkerMessage::BookSplit(BatchSplitProgress::Finished(
        outcome,
    )))
    .unwrap();
    drop(tx);

    app.poll_worker();

    assert!(matches!(
        app.splitter.batch_queue.statuses[0],
        splitter::BatchSplitStatus::Done(2)
    ));
    assert!(matches!(
        app.splitter.batch_queue.statuses[1],
        splitter::BatchSplitStatus::Failed(_)
    ));
    assert!(!app.splitter.batch_queue.running);
    assert!(!app.worker.is_busy);
    assert!(app.worker.cancel_flag.is_none());
    assert_eq!(app.splitter.batch_queue.summary, Some(outcome));
    assert_eq!(app.worker.progress_ratio, 1.0);
    assert_eq!(
        app.view.status_line,
        "Пакетный книгорез: 3 частей, ошибок 2"
    );
    assert_eq!(app.view.status_level, StatusLevel::Warning);

    let mut harness = Harness::builder()
        .with_size(egui::vec2(1100.0, 800.0))
        .build_ui_state(|ui, app: &mut DemagogApp| app.update_ui(ui), app);
    harness.run();
    assert_eq!(harness.get_all_by_label("Готово: 3, ошибок: 2").count(), 1);
}

#[test]
fn reset_button_returns_app_to_launch_state() {
    use crate::domain::{NormalizationStats, ReplacementEvent, StepReport};
    use egui_kittest::Harness;
    use egui_kittest::kittest::Queryable;

    let mut app = ui_test_app("Текст", "Результат", Vec::new());
    app.editor.raw_pipeline_output = "Вывод конвейера".to_string();
    app.editor.source_dirty = true;
    app.editor.chunk_apply_pending = true;
    app.editor
        .result_state
        .mark_computed(StressFormat::CleanYoOnly);
    app.editor.result_state.mark_edited();
    app.editor.result_state.mark_stale();
    app.processing.report = PipelineReport {
        steps: vec![StepReport {
            dictionary_name: "dict".to_string(),
            kind: DictionaryKind::DicStandard,
            rule_count: 2,
            elapsed_ms: 3,
            replacements_count: 4,
        }],
        total_ms: 5,
        total_replacements: 6,
        events: vec![ReplacementEvent {
            dictionary_name: "dict".to_string(),
            rule_summary: "rule".to_string(),
            before_snippet: "before".to_string(),
            after_snippet: "after".to_string(),
        }],
        events_truncated: true,
        cleanup: NormalizationStats {
            removed_empty_lines: 7,
            added_periods: 8,
        },
    };
    app.processing.processing_snapshot = Some(ProcessingSnapshot::capture("Текст"));
    app.processing.capture_diff_on_run = true;
    app.splitter
        .batch_queue
        .add_files([std::env::temp_dir().join("demagog-reset-test.txt")]);
    app.view.active_tab = ActiveTab::DiffAndStats;
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1280.0, 900.0))
        .build_ui_state(|ui, app: &mut DemagogApp| app.update_ui(ui), app);
    harness.run();
    harness.get_by_label("🔄 Сброс").click();
    harness.run_steps(2);
    let state = harness.state();
    assert_eq!(state.editor.document.title, "Пример текста");
    assert!(state.editor.document.source_path.is_none());
    assert!(state.editor.document.raw_text.contains("В 1995 году"));
    assert!(state.editor.document.processed_text.is_empty());
    assert_eq!(state.editor.processed_char_count, 0);
    assert!(state.editor.raw_pipeline_output.is_empty());
    assert!(!state.editor.source_dirty);
    assert!(!state.editor.chunk_apply_pending);
    assert!(!state.editor.result_state.is_dirty());
    assert!(!state.editor.result_state.is_stale());
    assert!(!state.editor.result_state.is_edited());
    assert!(!state.editor.result_state.is_chunks_stale());
    assert_eq!(
        state.editor.result_state.format(),
        StressFormat::CleanYoOnly
    );
    assert!(state.processing.report.steps.is_empty());
    assert_eq!(state.processing.report.total_ms, 0);
    assert_eq!(state.processing.report.total_replacements, 0);
    assert!(state.processing.report.events.is_empty());
    assert!(!state.processing.report.events_truncated);
    assert_eq!(
        state.processing.report.cleanup,
        NormalizationStats::default()
    );
    assert!(state.processing.processing_snapshot.is_none());
    assert!(!state.processing.capture_diff_on_run);
    assert_eq!(state.view.active_tab, ActiveTab::RawText);
    assert!(state.splitter.batch_queue.files.is_empty());
}

#[test]
fn reset_app_preserves_window_title_and_icon() {
    let mut app = ui_test_app("Текст", "Результат", Vec::new());
    app.view.last_window_title = "Demagog-RS — Текст".to_string();
    app.view.window_icon_applied = true;

    app.reset_app(egui::Context::default());

    assert_eq!(app.view.last_window_title, "Demagog-RS — Текст");
    assert!(app.view.window_icon_applied);
}

#[test]
fn knigorez_headings_invalidated_when_source_text_cleared() {
    use crate::domain::BookHeading;
    use egui::accesskit::Role;
    use egui_kittest::Harness;
    use egui_kittest::kittest::{NodeT, Queryable};

    let mut app = ui_test_app("Глава 1\nТекст главы.", "", Vec::new());
    app.editor.document.headings = vec![BookHeading {
        offset: 0,
        title: "Глава 1".to_string(),
    }];
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1280.0, 900.0))
        .build_ui_state(|ui, app: &mut DemagogApp| app.update_ui(ui), app);
    harness.run();

    harness.get_by_label("🗑 Очистить").click();
    harness.run_steps(1);
    assert!(harness.state().editor.document.raw_text.is_empty());
    assert!(
        harness.state().editor.document.headings.is_empty(),
        "headings must be invalidated when the source text is cleared"
    );

    harness.get_by_role(Role::MultilineTextInput).click();
    harness.run_steps(1);
    harness
        .get_by_role(Role::MultilineTextInput)
        .type_text("Новый текст без структуры глав");
    harness.run();
    assert_eq!(
        harness.state().editor.document.raw_text,
        "Новый текст без структуры глав"
    );
    assert!(
        harness.state().editor.document.headings.is_empty(),
        "headings must stay invalidated after editing the source text"
    );

    harness.get_by_label("🔪 Книгорез").click();
    harness.run();
    assert_eq!(
        harness
            .get_all_by_label_contains("В этой книге нет структуры глав")
            .count(),
        1
    );
    assert!(
        harness
            .get_by_label_contains("По оглавлению")
            .accesskit_node()
            .is_disabled(),
        "TOC checkbox must be disabled when the book has no headings"
    );
}
