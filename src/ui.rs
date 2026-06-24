use rfd::MessageDialog;
use std::path::Path;

use slint::{SharedPixelBuffer, Rgba8Pixel};
use qrcode::QrCode;

use crate::games::GAMES;
use crate::AppWindow;

pub fn set_install_indicators(ui: &AppWindow, path: &str) {
    let base = Path::new(path);

    ui.set_inst_base(base.join("WaitTitleProject.exe").exists());

    for g in GAMES.iter().filter(|g| !g.exe_indicator.is_empty()) {
        match g.inst_property {
            "inst_kh1" => ui.set_inst_kh1(base.join(g.exe_indicator).exists()),
            "inst_recom" => ui.set_inst_recom(base.join(g.exe_indicator).exists()),
            "inst_kh2" => ui.set_inst_kh2(base.join(g.exe_indicator).exists()),
            "inst_bbs" => ui.set_inst_bbs(base.join(g.exe_indicator).exists()),
            "inst_theater" => ui.set_inst_theater(base.join(g.exe_indicator).exists()),
            _ => {}
        }
    }

    let mare_path = base.join("KINGDOM HEARTS HD 1.5+2.5 Launcher.exe");
    ui.set_inst_mare(mare_path.exists());

    let days_installed = base.join("STEAM/Mare/MOVIE/Days").exists()
        || base.join("EPIC/Mare/MOVIE/Days").exists();
    let coded_installed = base.join("STEAM/Mare/MOVIE/ReCoded").exists()
        || base.join("EPIC/Mare/MOVIE/ReCoded").exists();
    ui.set_inst_days(days_installed);
    ui.set_inst_coded(coded_installed);
}

pub fn set_login_ui(
    weak_ui: &slint::Weak<AppWindow>,
    status: &str,
    show_guard: bool,
    show_qr: bool,
    show_mobile_confirm: bool,
    busy: bool,
    qr_url: Option<&str>,
) {
    let status = status.to_string();
    let qr_buffer = qr_url.map(generate_qr_buffer);

    let weak = weak_ui.clone();
    let _ = slint::invoke_from_event_loop(move || {
        if let Some(ui) = weak.upgrade() {
            ui.set_login_status(status.into());
            ui.set_login_show_guard(show_guard);
            ui.set_login_show_qr(show_qr);
            ui.set_login_show_mobile_confirm(show_mobile_confirm);
            ui.set_login_busy(busy);

            if let Some(buf) = qr_buffer {
                ui.set_login_qr_image(slint::Image::from_rgba8(buf));
            }
        }
    });
}

pub fn update_status(weak_ui: &slint::Weak<AppWindow>, msg: &str) {
    let msg = msg.to_string();
    let weak = weak_ui.clone();
    let _ = slint::invoke_from_event_loop(move || {
        if let Some(ui) = weak.upgrade() { ui.set_display_info(msg.into()); }
    });
}

pub fn show_error(msg: &str) {
    let msg = msg.to_string();
    let _ = slint::invoke_from_event_loop(move || {
        MessageDialog::new().set_level(rfd::MessageLevel::Error)
            .set_title("Error").set_description(&msg).show();
    });
}

pub fn detect_steam_install() -> Option<std::path::PathBuf> {
    #[cfg(windows)]
    {
        for key in &[r"HKLM\SOFTWARE\WOW6432Node\Valve\Steam", r"HKLM\SOFTWARE\Valve\Steam"] {
            if let Ok(out) = std::process::Command::new("reg").args(["query", key, "/v", "InstallPath"]).output() {
                for line in String::from_utf8_lossy(&out.stdout).lines() {
                    if line.trim_start().starts_with("InstallPath") {
                        if let Some(p) = line.splitn(4, "    ").last() {
                            let path = std::path::PathBuf::from(p.trim());
                            if path.exists() { return Some(path); }
                        }
                    }
                }
            }
        }
        None
    }
    #[cfg(not(windows))]
    {
        let home = std::env::var("HOME").ok()?;
        for s in &[
            "/.steam/steam",
            "/.local/share/Steam",
            "/.var/app/com.valvesoftware.Steam/.local/share/Steam",
        ] {
            let p = std::path::PathBuf::from(format!("{home}{s}"));
            if p.exists() { return Some(p); }
        }
        None
    }
}

pub fn generate_qr_buffer(url: &str) -> SharedPixelBuffer<Rgba8Pixel> {
    const PIXELS_PER_MODULE: usize = 6;

    let code = QrCode::new(url).unwrap();
    let module_count = code.width();
    let pixel_size = (module_count * PIXELS_PER_MODULE) as u32;

    let mut pixel_buffer = SharedPixelBuffer::<Rgba8Pixel>::new(pixel_size, pixel_size);
    let pixels = pixel_buffer.make_mut_slice();

    for y in 0..module_count {
        for x in 0..module_count {
            let is_dark = code[(x, y)] == qrcode::Color::Dark;
            let color = if is_dark {
                slint::Rgba8Pixel { r: 0, g: 0, b: 0, a: 255 }
            } else {
                slint::Rgba8Pixel { r: 255, g: 255, b: 255, a: 255 }
            };

            for dy in 0..PIXELS_PER_MODULE {
                for dx in 0..PIXELS_PER_MODULE {
                    let out_y = y * PIXELS_PER_MODULE + dy;
                    let out_x = x * PIXELS_PER_MODULE + dx;
                    let idx = (out_y * pixel_size as usize) + out_x;
                    pixels[idx] = color;
                }
            }
        }
    }
    pixel_buffer
}