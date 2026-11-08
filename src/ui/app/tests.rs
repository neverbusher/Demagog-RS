use super::*;

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
        source_path: None,
        raw_text: raw_text.to_string(),
        processed_text: processed_text.to_string(),
        chunks,
    };
    let mut app = DemagogApp::from_parts(AppSettings::default(), document);
    app.window_icon_applied = true;
    app
}

#[test]
fn diff_events_render_only_the_visible_rows() {
    use crate::domain::{DictionaryKind, ReplacementEvent, StepReport};
    use egui_kittest::Harness;
    use egui_kittest::kittest::Queryable;

    let mut app = ui_test_app("", "Результат", Vec::new());
    app.active_tab = ActiveTab::DiffAndStats;
    app.report.events = (0..1000)
        .map(|index| ReplacementEvent {
            dictionary_name: format!("dict-{index}"),
            rule_summary: format!("rule-{index}"),
            before_snippet: format!("before-{index}"),
            after_snippet: format!("after-{index}"),
        })
        .collect();
    app.report.steps = vec![StepReport {
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
    app.active_tab = ActiveTab::Chunks;
    app.result_state
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
    })))
    .unwrap();

    let mut app = ui_test_app("", "Результат", Vec::new());
    app.rx = Some(rx);
    app.is_busy = true;
    app.cancel_flag = Some(Arc::new(AtomicBool::new(false)));
    app.poll_worker();

    assert!(app.cancel_flag.is_none());
    assert!(!app.is_busy);
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
    app.active_tab = ActiveTab::Settings;
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
    app.active_tab = ActiveTab::Settings;
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
    app.is_busy = true;
    let cancel = Arc::new(AtomicBool::new(false));
    app.cancel_flag = Some(Arc::clone(&cancel));

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
    app.is_busy = true;
    app.cancel_flag = Some(Arc::new(AtomicBool::new(false)));
    let (tx, rx) = mpsc::channel();
    tx.send(WorkerMessage::BookLoaded(Ok(BookDocument {
        title: "Тест".to_string(),
        source_path: None,
        raw_text: "Текст".to_string(),
        processed_text: String::new(),
        chunks: Vec::new(),
    })))
    .unwrap();
    app.rx = Some(rx);

    app.poll_worker();

    assert!(app.cancel_flag.is_none());
    assert!(!app.is_busy);
}

#[test]
fn settings_about_is_scrollable_at_min_height() {
    use egui_kittest::Harness;
    use egui_kittest::kittest::Queryable;

    let mut app = ui_test_app("", "", Vec::new());
    app.active_tab = ActiveTab::Settings;
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
fn journal_heading_stays_visible_with_many_steps() {
    use crate::domain::{DictionaryKind, ReplacementEvent, StepReport};
    use egui_kittest::Harness;
    use egui_kittest::kittest::Queryable;

    let mut app = ui_test_app("", "Результат", Vec::new());
    app.active_tab = ActiveTab::DiffAndStats;
    app.report.steps = (0..40)
        .map(|i| StepReport {
            dictionary_name: format!("dict-{i}"),
            kind: DictionaryKind::DicStandard,
            rule_count: 10,
            elapsed_ms: 1,
            replacements_count: 1,
        })
        .collect();
    app.report.events = (0..120)
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
    app.active_tab = ActiveTab::Chunks;
    app.result_state
        .mark_computed(StressFormat::PlusBeforeVowel);
    app.result_state.mark_edited();

    let mut harness = Harness::builder()
        .with_size(egui::vec2(1280.0, 900.0))
        .build_ui_state(|ui, app: &mut DemagogApp| app.update_ui(ui), app);
    harness.run();
    harness.get_by_label_contains("Пересобрать чанки").click();
    harness.run();

    assert!(!harness.state().document.chunks.is_empty());
    assert!(!harness.state().result_state.is_chunks_stale());
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
    app.is_busy = true;

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

    harness.state_mut().is_busy = false;
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
    use crate::services::dictionary_audit::{
        AuditCandidate, AuditKind, AuditReport, DictionaryAudit,
    };
    use crate::services::dictionary_check::{
        CheckReport, GOLDEN_CASES, OrderReport, OrderedDictionary,
    };
    use egui_kittest::Harness;
    use egui_kittest::kittest::Queryable;

    let order = OrderReport {
        ordered: vec![OrderedDictionary {
            name: "10_REX_числа(chisla).rex".to_string(),
            stage: "REX",
        }],
        warnings: Vec::new(),
        failures: Vec::new(),
    };
    let audit = AuditReport {
        dictionaries: vec![DictionaryAudit {
            name: "67_TTS_Offline АВТОР(phoneme).dic".to_string(),
            purpose: "автор",
            total_rules: 10,
            candidates: vec![AuditCandidate {
                line: 1,
                lhs: "Всьо".to_string(),
                rhs: "Всё".to_string(),
                kind: AuditKind::Letters,
            }],
        }],
        skipped: vec![("65_ЛАТИНИЦА@.dic".to_string(), "транслитерация")],
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
    app.active_tab = ActiveTab::Settings;
    app.check_report = Some(report);
    app.show_check_window = true;

    let mut harness = Harness::builder()
        .with_size(egui::vec2(1280.0, 900.0))
        .build_ui_state(|ui, app: &mut DemagogApp| app.update_ui(ui), app);
    harness.run();

    harness.get_by_label_contains("Проверить словари");
    harness.get_by_label_contains("🔎 Проверка словарей");
    harness.get_by_label_contains("Фонетические кандидаты");
    harness.get_by_label_contains("Кандидатов: 1 из 10");
    harness.get_by_label_contains("67_TTS_Offline АВТОР(phoneme).dic");
}

#[test]
#[ignore]
fn clicking_dictionary_check_loads_report() {
    use egui_kittest::Harness;
    use egui_kittest::kittest::Queryable;

    let mut app = ui_test_app("", "", Vec::new());
    app.active_tab = ActiveTab::Settings;
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1280.0, 900.0))
        .build_ui_state(|ui, app: &mut DemagogApp| app.update_ui(ui), app);
    harness.run();

    harness.get_by_label_contains("Проверить словари").click();
    harness.run_steps(1);

    let deadline = Instant::now() + Duration::from_secs(60);
    while harness.state().check_report.is_none() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(50));
        harness.run_steps(1);
    }

    assert!(
        harness.state().check_report.is_some(),
        "dictionary check did not finish within 60 seconds"
    );
    assert!(harness.state().show_check_window);
    harness.get_by_label_contains("Фонетические кандидаты");
}
