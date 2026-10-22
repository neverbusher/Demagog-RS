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
