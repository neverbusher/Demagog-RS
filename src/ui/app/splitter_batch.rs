//! Пакетный режим вкладки «Книгорез»: очередь, статусы, запуск нарезки.

use std::collections::HashSet;

use super::splitter::BatchSplitStatus;
use super::*;
use crate::services::book_splitter::{
    self, BatchSplitProgress, run_split_batch_with_confirmation, validate_config,
};

impl DemagogApp {
    /// Пакетное меню — очередь книг, статусы, запуск пакетной нарезки.
    pub(super) fn show_splitter_batch_menu(&mut self, ui: &mut egui::Ui, theme: ThemeMode) {
        let changed = self.show_splitter_settings(ui);
        self.persist_split_change(ui, changed);
        if changed {
            self.splitter.batch_queue.warning = None;
            if validate_config(&self.settings.split).is_ok() {
                self.splitter.batch_queue.error = None;
            }
        }
        self.show_batch_queue_section(ui, theme);
    }

    fn show_batch_queue_section(&mut self, ui: &mut egui::Ui, theme: ThemeMode) {
        ui.label("Пакетная нарезка");
        ui.horizontal_wrapped(|ui| {
            if ui
                .add_enabled(
                    !self.worker.is_busy,
                    egui::Button::new("➕ Добавить файлы…"),
                )
                .on_hover_text("Выбрать несколько книг (TXT, FB2, EPUB, DOCX, ZIP)")
                .clicked()
                && let Some(paths) = rfd::FileDialog::new()
                    .add_filter(
                        "Книги и текст",
                        &["txt", "fb2", "epub", "docx", "zip", "md", "log"],
                    )
                    .pick_files()
            {
                self.splitter.batch_queue.add_files(paths);
            }
            if ui
                .add_enabled(
                    !self.splitter.batch_queue.running
                        && !self.splitter.batch_queue.files.is_empty(),
                    egui::Button::new("🗑 Очистить"),
                )
                .clicked()
            {
                self.splitter.batch_queue.clear();
            }
        });
        if self.splitter.batch_queue.files.is_empty() {
            let hint = if self.settings.split.subfolder {
                "Добавьте книги — каждая будет нарезана в папке «ИмяКниги»."
            } else {
                "Добавьте книги — части будут записаны в выбранную папку."
            };
            ui.label(hint);
        }
        ScrollArea::vertical()
            .id_salt("splitter_batch_queue")
            .max_height(180.0)
            .show(ui, |ui| {
                for (index, status) in self.splitter.batch_queue.statuses.iter().enumerate() {
                    let (icon, detail) = match status {
                        BatchSplitStatus::Queued => ("⏳", String::new()),
                        BatchSplitStatus::Running => ("⚙", "нарезка…".to_string()),
                        BatchSplitStatus::Done(parts) => ("✅", format!("{parts} частей")),
                        BatchSplitStatus::Partial { saved, error } => {
                            ("⚠", format!("сохранено {saved} частей; {error}"))
                        }
                    };
                    let name = self.splitter.batch_queue.display_name(index);
                    ui.horizontal_wrapped(|ui| {
                        ui.label(icon);
                        ui.label(name);
                        if !detail.is_empty() {
                            ui.label(RichText::new(detail).small().weak());
                        }
                    });
                }
            });
        let total = self.splitter.batch_queue.statuses.len();
        if self.splitter.batch_queue.running && total > 0 {
            let done =
                self.splitter.batch_queue.done_count() + self.splitter.batch_queue.failed_count();
            ui.add(
                egui::ProgressBar::new(done as f32 / total as f32).text(format!("{done}/{total}")),
            );
        }
        ui.horizontal_wrapped(|ui| {
            if self.splitter.batch_queue.running {
                let label = if self.splitter.batch_queue.cancel_requested {
                    "⏹ Отмена…"
                } else {
                    "⏹ Отмена"
                };
                if ui
                    .add_enabled(
                        !self.splitter.batch_queue.cancel_requested,
                        egui::Button::new(label),
                    )
                    .clicked()
                {
                    self.splitter.batch_queue.cancel_requested = true;
                    self.splitter.batch_queue.warning =
                        Some("Отмена запрошена; ожидаем безопасной остановки…".to_string());
                    if let Some(flag) = &self.worker.cancel_flag {
                        flag.store(true, Ordering::Relaxed);
                    }
                }
            } else if ui
                .add_enabled(
                    self.splitter.batch_queue.can_start(
                        self.worker.is_busy,
                        self.settings.split.output_dir.is_some(),
                    ),
                    egui::Button::new("▶ Нарезать пакет"),
                )
                .on_disabled_hover_text("Добавьте файлы и выберите папку назначения")
                .clicked()
            {
                self.start_batch_split(ui.ctx().clone());
            }
            if let Some(outcome) = self.splitter.batch_queue.summary {
                let counts = outcome.counts();
                let text = if outcome.is_cancelled() {
                    format!(
                        "Отменено: книг {}, ошибок {}",
                        counts.succeeded, counts.failed
                    )
                } else {
                    format!(
                        "Готово: книг {}, ошибок: {}",
                        counts.succeeded, counts.failed
                    )
                };
                ui.label(RichText::new(text).strong());
            }
        });
        if let Some(warning) = &self.splitter.batch_queue.warning {
            ui.label(RichText::new(warning.clone()).color(palette::warning_text(theme)));
        }
        if let Some(error) = &self.splitter.batch_queue.error {
            ui.label(RichText::new(error).color(ui.visuals().error_fg_color));
        }
    }

    /// Папки «ИмяКниги» для всех в очереди пакетной нарезки, уже существующие на диске.
    fn batch_existing_dirs(&self) -> Vec<String> {
        let Some(base) = self.settings.split.output_dir.as_ref() else {
            return Vec::new();
        };
        if !self.settings.split.overwrite {
            return Vec::new();
        }
        if !self.settings.split.subfolder {
            return if base.exists() {
                vec![base.display().to_string()]
            } else {
                Vec::new()
            };
        }
        let mut used_dirs = HashSet::new();
        let mut dirs: Vec<String> = self
            .splitter
            .batch_queue
            .files
            .iter()
            .map(|file| {
                let source_name = crate::services::batch::output_stem(&file.path);
                book_splitter::unique_book_dir(base, &source_name, &mut used_dirs)
            })
            .filter(|dir| dir.exists())
            .map(|dir| dir.display().to_string())
            .collect();
        dirs.sort();
        dirs.dedup();
        dirs
    }

    fn start_batch_split(&mut self, ctx: egui::Context) {
        if !self.splitter.batch_queue.can_start(
            self.worker.is_busy,
            self.settings.split.output_dir.is_some(),
        ) {
            return;
        }
        if let Err(err) = validate_config(&self.settings.split) {
            self.splitter.batch_queue.error = Some(err.to_string());
            return;
        }
        self.splitter.batch_queue.warning = None;
        let existing = self.batch_existing_dirs();
        if !existing.is_empty() {
            let warning = format!(
                "Папки уже существуют: {} — файлы внутри могут затереться. Продолжить?",
                existing.join(", ")
            );
            if !confirm_destructive(&warning) {
                return;
            }
            self.splitter.batch_queue.warning = Some(warning);
        }
        self.splitter
            .batch_queue
            .statuses
            .fill(BatchSplitStatus::Queued);
        self.splitter.batch_queue.summary = None;
        self.splitter.batch_queue.cancel_requested = false;
        self.splitter.batch_queue.error = None;
        self.splitter.batch_queue.running = true;
        self.worker.is_busy = true;
        self.worker.progress_ratio = 0.0;

        let files = self.splitter.batch_queue.files.clone();
        let config = self.settings.split.clone();
        let cancel = Arc::new(AtomicBool::new(false));
        self.worker.cancel_flag = Some(Arc::clone(&cancel));
        let (tx, rx) = mpsc::channel();
        self.worker.rx = Some(rx);
        let (confirm_tx, confirm_rx) = mpsc::channel();
        self.worker.split_confirm_tx = Some(confirm_tx);

        std::thread::spawn(move || {
            let (btx, brx) = mpsc::channel();
            let tx_forward = tx.clone();
            let ctx_forward = ctx.clone();
            let forwarder = std::thread::spawn(move || {
                while let Ok(msg) = brx.recv() {
                    let _ = tx_forward.send(WorkerMessage::BookSplit(msg));
                    ctx_forward.request_repaint();
                }
            });
            let confirm_tx = btx.clone();
            let confirm = move |parts: usize| {
                if confirm_tx
                    .send(BatchSplitProgress::ConfirmLargeExport { parts })
                    .is_err()
                {
                    return false;
                }
                confirm_rx.recv().unwrap_or(false)
            };
            let reader = CompositeBookReader::new();
            run_split_batch_with_confirmation(
                &files,
                &reader,
                &config,
                Some(cancel.as_ref()),
                Some(&btx),
                Some(&confirm),
            );
            drop(confirm);
            drop(btx);
            let _ = forwarder.join();
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "demagog-split-ui-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn changing_settings_clears_stale_batch_messages() {
        use egui_kittest::Harness;
        use egui_kittest::kittest::Queryable;

        let root = temp_dir("stale-messages");
        let mut app = DemagogApp::from_parts(AppSettings::default(), BookDocument::default());
        app.settings.split.output_dir = Some(root.clone());
        app.settings.split.by_size = false;
        app.splitter.batch_queue.add_files([root.join("book.txt")]);
        app.splitter.batch_queue.error = Some("старая ошибка".to_string());
        app.splitter.batch_queue.warning = Some("старое предупреждение".to_string());
        app.view.active_tab = ActiveTab::BookCutter;
        let mut harness = Harness::builder()
            .with_size(egui::vec2(1100.0, 800.0))
            .build_ui_state(|ui, app: &mut DemagogApp| app.update_ui(ui), app);

        harness.get_by_label("По размеру").click();
        harness.run_steps(1);

        assert!(harness.state().splitter.batch_queue.error.is_none());
        assert!(harness.state().splitter.batch_queue.warning.is_none());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn flat_batch_warning_checks_the_real_output_directory() {
        let root = temp_dir("flat-warning");
        let mut app = DemagogApp::from_parts(AppSettings::default(), BookDocument::default());
        app.settings.split.output_dir = Some(root.clone());
        app.settings.split.subfolder = false;
        app.settings.split.overwrite = true;
        app.splitter.batch_queue.add_files([root.join("book.txt")]);

        assert_eq!(app.batch_existing_dirs(), vec![root.display().to_string()]);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn batch_warning_accounts_for_same_stem_suffix_folder() {
        let root = temp_dir("suffix-warning");
        std::fs::create_dir_all(root.join("book (2)")).unwrap();
        let mut app = DemagogApp::from_parts(AppSettings::default(), BookDocument::default());
        app.settings.split.output_dir = Some(root.clone());
        app.settings.split.subfolder = true;
        app.settings.split.overwrite = true;
        app.splitter.batch_queue.add_files([
            root.join("first").join("book.txt"),
            root.join("second").join("book.txt"),
        ]);

        assert_eq!(
            app.batch_existing_dirs(),
            vec![root.join("book (2)").display().to_string()]
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn batch_warning_is_empty_when_overwrite_is_disabled() {
        let root = temp_dir("no-overwrite-warning");
        std::fs::create_dir_all(root.join("book")).unwrap();
        let mut app = DemagogApp::from_parts(AppSettings::default(), BookDocument::default());
        app.settings.split.output_dir = Some(root.clone());
        app.settings.split.subfolder = true;
        app.settings.split.overwrite = false;
        app.splitter.batch_queue.add_files([root.join("book.txt")]);

        assert!(app.batch_existing_dirs().is_empty());
        let _ = std::fs::remove_dir_all(&root);
    }
}
