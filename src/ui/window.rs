use egui::ViewportBuilder;

#[derive(Debug, Clone, Copy, PartialEq)]
struct WorkArea {
    x: f32,
    y: f32,
    width: f32,
    height: f32,
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct WindowMetrics {
    width: f32,
    height: f32,
    min_width: f32,
    min_height: f32,
    x: f32,
    y: f32,
}

pub fn initial_viewport() -> ViewportBuilder {
    match work_area() {
        Some(area) => viewport_for_work_area(area),
        None => fallback_viewport(),
    }
}

fn fallback_viewport() -> ViewportBuilder {
    ViewportBuilder::default()
        .with_title("Demagog-RS")
        .with_inner_size([1440.0, 900.0])
        .with_min_inner_size([1000.0, 640.0])
}

fn viewport_for_work_area(area: WorkArea) -> ViewportBuilder {
    let metrics = fit_to_work_area(area.width, area.height);
    ViewportBuilder::default()
        .with_title("Demagog-RS")
        .with_inner_size([metrics.width, metrics.height])
        .with_min_inner_size([metrics.min_width, metrics.min_height])
        .with_position(egui::pos2(area.x + metrics.x, area.y + metrics.y))
}

fn fit_to_work_area(work_width: f32, work_height: f32) -> WindowMetrics {
    // Reserve room for the title bar and borders: the work area describes the
    // outer window rect, while `inner_size` is the client area.
    let avail_width = (work_width - 16.0).max(320.0);
    let avail_height = (work_height - 40.0).max(240.0);

    let target_width = (avail_width * 0.9).min(1440.0);
    let target_height = (avail_height * 0.9).min(900.0);
    let min_width = 1000.0_f32.min(avail_width);
    let min_height = 640.0_f32.min(avail_height);

    let width = target_width.max(min_width).min(avail_width);
    let height = target_height.max(min_height).min(avail_height);

    WindowMetrics {
        width,
        height,
        min_width: min_width.min(width),
        min_height: min_height.min(height),
        x: (work_width - width) / 2.0,
        y: (work_height - height) / 2.0,
    }
}

#[cfg(windows)]
fn work_area() -> Option<WorkArea> {
    #[repr(C)]
    struct Rect {
        left: i32,
        top: i32,
        right: i32,
        bottom: i32,
    }

    #[link(name = "user32")]
    unsafe extern "system" {
        fn SystemParametersInfoW(action: u32, param: u32, data: *mut Rect, win_ini: u32) -> i32;
        fn GetDpiForSystem() -> u32;
    }

    const SPI_GETWORKAREA: u32 = 0x0030;

    let mut rect = Rect {
        left: 0,
        top: 0,
        right: 0,
        bottom: 0,
    };
    let ok = unsafe { SystemParametersInfoW(SPI_GETWORKAREA, 0, &mut rect, 0) };
    if ok == 0 {
        return None;
    }

    let dpi = unsafe { GetDpiForSystem() };
    let scale = if dpi > 0 { dpi as f32 / 96.0 } else { 1.0 };
    let width = (rect.right - rect.left) as f32 / scale;
    let height = (rect.bottom - rect.top) as f32 / scale;
    if width < 320.0 || height < 240.0 {
        return None;
    }

    Some(WorkArea {
        x: rect.left as f32 / scale,
        y: rect.top as f32 / scale,
        width,
        height,
    })
}

#[cfg(not(windows))]
fn work_area() -> Option<WorkArea> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_fits(work_width: f32, work_height: f32) -> WindowMetrics {
        let metrics = fit_to_work_area(work_width, work_height);
        assert!(metrics.width <= work_width);
        assert!(metrics.height <= work_height);
        assert!(metrics.min_width <= metrics.width);
        assert!(metrics.min_height <= metrics.height);
        assert!(metrics.x >= 0.0);
        assert!(metrics.y >= 0.0);
        metrics
    }

    #[test]
    fn uses_most_of_a_1600x900_screen_but_fits() {
        let metrics = assert_fits(1600.0, 860.0);
        assert!((metrics.width - 1425.6).abs() < 0.1);
        assert!((metrics.height - 738.0).abs() < 0.1);
    }

    #[test]
    fn caps_large_screens_at_1440x900() {
        let metrics = assert_fits(1920.0, 1040.0);
        assert!((metrics.width - 1440.0).abs() < 0.1);
        assert!((metrics.height - 900.0).abs() < 0.1);
    }

    #[test]
    fn respects_small_work_areas() {
        let metrics = assert_fits(1366.0, 688.0);
        assert!((metrics.height - 640.0).abs() < 0.1);
        let tiny = assert_fits(1024.0, 560.0);
        assert!((tiny.width - 1000.0).abs() < 0.1);
        assert!((tiny.height - 520.0).abs() < 0.1);
    }
}
