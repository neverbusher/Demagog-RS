//! CPU-only runtime: software rasterization via `egui_software_backend`,
//! window presentation via `softbuffer` (GDI on Windows).
//! Works on machines without OpenGL, DirectX 12 or any GPU driver.

use std::path::PathBuf;

use egui_software_backend::{
    App as SwApp, SoftwareBackend, SoftwareBackendAppConfiguration,
};

use super::app::DemagogApp;

pub fn run(initial_dic_folder: PathBuf) -> Result<(), Box<dyn std::error::Error>> {
    let config = SoftwareBackendAppConfiguration::new()
        .title(Some("Demagog-RS".to_owned()))
        .inner_size(Some(egui::vec2(1440.0, 900.0)))
        .min_inner_size(Some(egui::vec2(1000.0, 640.0)));

    // The factory may be called again if the window is suspended and resumed,
    // so the dictionary folder is cloned per invocation.
    egui_software_backend::run_app_with_software_backend(config, move |ctx| SwDemagogApp {
        app: DemagogApp::init(ctx, initial_dic_folder.clone()),
    })?;
    Ok(())
}

struct SwDemagogApp {
    app: DemagogApp,
}

impl SwApp for SwDemagogApp {
    fn ui(&mut self, ui: &mut egui::Ui, _backend: &mut SoftwareBackend) {
        self.app.update_ui(ui);
    }

    fn on_exit(&mut self, _ctx: &egui::Context) {
        self.app.shutdown();
    }
}
