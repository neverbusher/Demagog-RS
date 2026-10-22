use std::sync::{Arc, OnceLock};

use egui::{IconData, ViewportBuilder};

static APP_ICON: OnceLock<Arc<IconData>> = OnceLock::new();

pub(crate) fn app_icon() -> Arc<IconData> {
    APP_ICON
        .get_or_init(|| {
            let bytes = include_bytes!("../../assets/icon.png");
            let decoder = png::Decoder::new(std::io::Cursor::new(bytes));
            let mut reader = decoder
                .read_info()
                .expect("bundled app icon must be a valid PNG");
            let mut rgba = vec![0; reader.output_buffer_size()];
            let info = reader
                .next_frame(&mut rgba)
                .expect("bundled app icon must decode");
            rgba.truncate(info.buffer_size());
            assert_eq!(info.color_type, png::ColorType::Rgba);
            assert_eq!(info.bit_depth, png::BitDepth::Eight);
            Arc::new(IconData {
                rgba,
                width: info.width,
                height: info.height,
            })
        })
        .clone()
}

/// The software backend only calls winit's `set_window_icon`, which sets
/// `ICON_SMALL`. Set both sizes explicitly so the taskbar and Alt+Tab match
/// the glow build.
#[cfg(windows)]
pub(crate) fn apply_window_icon(icon: &IconData) -> bool {
    use std::cell::Cell;
    use std::ffi::c_void;
    use std::ptr::null_mut;

    type Hwnd = *mut c_void;
    type Hbitmap = *mut c_void;
    type Hdc = *mut c_void;
    type Hicon = *mut c_void;

    #[repr(C)]
    struct BitmapInfoHeader {
        bi_size: u32,
        bi_width: i32,
        bi_height: i32,
        bi_planes: u16,
        bi_bit_count: u16,
        bi_compression: u32,
        bi_size_image: u32,
        bi_x_pels_per_meter: i32,
        bi_y_pels_per_meter: i32,
        bi_clr_used: u32,
        bi_clr_important: u32,
    }

    #[repr(C)]
    struct BitmapInfo {
        header: BitmapInfoHeader,
        colors: [u32; 1],
    }

    #[repr(C)]
    struct IconInfo {
        f_icon: i32,
        x_hotspot: u32,
        y_hotspot: u32,
        hbm_mask: Hbitmap,
        hbm_color: Hbitmap,
    }

    #[link(name = "user32")]
    unsafe extern "system" {
        fn EnumWindows(
            callback: unsafe extern "system" fn(Hwnd, isize) -> i32,
            lparam: isize,
        ) -> i32;
        fn GetWindowThreadProcessId(hwnd: Hwnd, pid: *mut u32) -> u32;
        fn IsWindowVisible(hwnd: Hwnd) -> i32;
        fn GetWindowTextW(hwnd: Hwnd, text: *mut u16, max_count: i32) -> i32;
        fn SendMessageW(hwnd: Hwnd, msg: u32, wparam: usize, lparam: isize) -> isize;
        fn GetDC(hwnd: Hwnd) -> Hdc;
        fn ReleaseDC(hwnd: Hwnd, hdc: Hdc) -> i32;
    }

    #[link(name = "gdi32")]
    unsafe extern "system" {
        fn CreateDIBSection(
            hdc: Hdc,
            bmi: *const BitmapInfo,
            usage: u32,
            bits: *mut *mut c_void,
            section: *mut c_void,
            offset: u32,
        ) -> Hbitmap;
        fn CreateBitmap(
            width: i32,
            height: i32,
            planes: u32,
            bit_count: u32,
            bits: *const c_void,
        ) -> Hbitmap;
        fn CreateIconIndirect(info: *const IconInfo) -> Hicon;
        fn DeleteObject(object: *mut c_void) -> i32;
    }

    thread_local! {
        static FOUND_WINDOW: Cell<Hwnd> = const { Cell::new(null_mut()) };
    }

    unsafe extern "system" fn find_main_window(hwnd: Hwnd, lparam: isize) -> i32 {
        let process_id = lparam as u32;
        let mut window_process_id = 0;
        unsafe { GetWindowThreadProcessId(hwnd, &mut window_process_id) };
        if window_process_id != process_id || unsafe { IsWindowVisible(hwnd) } == 0 {
            return 1;
        }
        let mut text = [0u16; 128];
        let len = unsafe { GetWindowTextW(hwnd, text.as_mut_ptr(), text.len() as i32) };
        if len > 0 {
            let title = String::from_utf16_lossy(&text[..len as usize]);
            if title.starts_with("Demagog-RS") {
                FOUND_WINDOW.with(|found| found.set(hwnd));
                return 0;
            }
        }
        1
    }

    FOUND_WINDOW.with(|found| found.set(null_mut()));
    unsafe { EnumWindows(find_main_window, std::process::id() as isize) };
    let hwnd = FOUND_WINDOW.with(Cell::get);
    if hwnd.is_null() {
        return false;
    }

    let hdc = unsafe { GetDC(hwnd) };
    if hdc.is_null() {
        return false;
    }

    let width = icon.width as i32;
    let height = icon.height as i32;
    let bmi = BitmapInfo {
        header: BitmapInfoHeader {
            bi_size: std::mem::size_of::<BitmapInfoHeader>() as u32,
            bi_width: width,
            bi_height: height,
            bi_planes: 1,
            bi_bit_count: 32,
            bi_compression: 0,
            bi_size_image: 0,
            bi_x_pels_per_meter: 0,
            bi_y_pels_per_meter: 0,
            bi_clr_used: 0,
            bi_clr_important: 0,
        },
        colors: [0],
    };
    let mut bits: *mut c_void = null_mut();
    let color = unsafe { CreateDIBSection(hdc, &bmi, 0, &mut bits, null_mut(), 0) };
    if color.is_null() || bits.is_null() {
        unsafe { ReleaseDC(hwnd, hdc) };
        return false;
    }

    let pixel_width = icon.width as usize;
    let pixel_height = icon.height as usize;
    let stride = pixel_width * 4;
    for y in 0..pixel_height {
        let source = &icon.rgba[(pixel_height - 1 - y) * stride..][..stride];
        let destination = unsafe { (bits as *mut u8).add(y * stride) };
        for x in 0..pixel_width {
            unsafe {
                *destination.add(x * 4) = source[x * 4 + 2];
                *destination.add(x * 4 + 1) = source[x * 4 + 1];
                *destination.add(x * 4 + 2) = source[x * 4];
                *destination.add(x * 4 + 3) = source[x * 4 + 3];
            }
        }
    }

    let mask_stride = pixel_width.div_ceil(16) * 2;
    let mask_bits = vec![0u8; mask_stride * pixel_height];
    let mask = unsafe { CreateBitmap(width, height, 1, 1, mask_bits.as_ptr() as *const c_void) };
    if mask.is_null() {
        unsafe {
            DeleteObject(color);
            ReleaseDC(hwnd, hdc);
        }
        return false;
    }

    let info = IconInfo {
        f_icon: 1,
        x_hotspot: 0,
        y_hotspot: 0,
        hbm_mask: mask,
        hbm_color: color,
    };
    let hicon = unsafe { CreateIconIndirect(&info) };
    unsafe {
        DeleteObject(color);
        DeleteObject(mask);
        ReleaseDC(hwnd, hdc);
    }
    if hicon.is_null() {
        return false;
    }

    const WM_SETICON: u32 = 0x0080;
    const ICON_SMALL: usize = 0;
    const ICON_BIG: usize = 1;
    unsafe {
        SendMessageW(hwnd, WM_SETICON, ICON_BIG, hicon as isize);
        SendMessageW(hwnd, WM_SETICON, ICON_SMALL, hicon as isize);
    }
    true
}

#[cfg(not(windows))]
pub(crate) fn apply_window_icon(_icon: &IconData) -> bool {
    true
}

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
    let builder = match work_area() {
        Some(area) => viewport_for_work_area(area),
        None => fallback_viewport(),
    };
    builder.with_icon(app_icon())
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
    fn bundled_icon_is_valid_rgba() {
        let icon = app_icon();
        assert_eq!(icon.width, 512);
        assert_eq!(icon.height, 512);
        assert_eq!(icon.rgba.len(), 512 * 512 * 4);
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
