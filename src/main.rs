#![recursion_limit = "256"]
slint::include_modules!();

use rfd::{FileDialog, MessageDialog, MessageLevel, MessageButtons, MessageDialogResult};
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use steamroom::depot::DepotId;

use tokio::sync::mpsc;

use fs4::available_space;

mod games;
mod ui;
mod uninstall;
mod steam;
mod epic;
mod permissions;

const LAUNCHERS_URL: &str = "https://github.com/SandeMC/Kingdom-Hearts-Launchers";

use games::GAMES;
use ui::{set_install_indicators, detect_steam_install};
use steam::auth::{AuthInput, run_install_flow};
use uninstall::run_uninstall;
use epic::auth::EpicAuthInput;

fn get_sel(ui: &AppWindow, prop: &str) -> bool {
    match prop {
        "sel_base"    => ui.get_sel_base(),
        "sel_mare"    => ui.get_sel_mare(),
        "sel_kh1"     => ui.get_sel_kh1(),
        "sel_recom"   => ui.get_sel_recom(),
        "sel_days"    => ui.get_sel_days(),
        "sel_kh2"     => ui.get_sel_kh2(),
        "sel_bbs"     => ui.get_sel_bbs(),
        "sel_coded"   => ui.get_sel_coded(),
        "sel_theater" => ui.get_sel_theater(),
        _ => false,
    }
}

fn set_name(ui: &AppWindow, g: &games::GameEntry, val: slint::SharedString) {
    match g.id {
        "base"    => ui.set_name_base(val),
        "mare"    => ui.set_name_mare(val),
        "kh1"     => ui.set_name_kh1(val),
        "recom"   => ui.set_name_recom(val),
        "days"    => ui.set_name_days(val),
        "kh2"     => ui.set_name_kh2(val),
        "bbs"     => ui.set_name_bbs(val),
        "coded"   => ui.set_name_coded(val),
        "theater" => ui.set_name_theater(val),
        _ => {}
    }
}

fn set_size(ui: &AppWindow, g: &games::GameEntry, val: f32) {
    match g.id {
        "base"    => ui.set_size_base(val),
        "mare"    => ui.set_size_mare(val),
        "kh1"     => ui.set_size_kh1(val),
        "recom"   => ui.set_size_recom(val),
        "days"    => ui.set_size_days(val),
        "kh2"     => ui.set_size_kh2(val),
        "bbs"     => ui.set_size_bbs(val),
        "coded"   => ui.set_size_coded(val),
        "theater" => ui.set_size_theater(val),
        _ => {}
    }
}

fn populate_name_sizes(ui: &AppWindow) {
    for g in GAMES {
        set_name(ui, g, g.display_name().into());
        set_size(ui, g, g.size_gib);
    }
}

fn steam_default_path() -> String {
    if let Some(steam_path) = detect_steam_install() {
        return steam_path
            .join("steamapps")
            .join("common")
            .join("KINGDOM HEARTS -HD 1.5+2.5 ReMIX-")
            .to_string_lossy()
            .to_string();
    }

    #[cfg(windows)]
    {
        "C:/Program Files (x86)/Steam/steamapps/common/KINGDOM HEARTS -HD 1.5+2.5 ReMIX-".to_string()
    }
    #[cfg(not(windows))]
    {
        let home = std::env::var("HOME").unwrap_or_default();
        format!("{home}/.local/share/Steam/steamapps/common/KINGDOM HEARTS -HD 1.5+2.5 ReMIX-")
    }
}

#[cfg(windows)]
fn epic_default_path() -> String {
    let drive = if Path::new("D:\\").exists() { "D:\\" } else { "C:\\" };
    format!("{drive}Games\\KH1.5_2.5")
}

#[cfg(not(windows))]
fn epic_default_path() -> String {
    let home = std::env::var("HOME").unwrap_or_default();
    format!("{home}\\Games\\Heroic\\Kingdom Hearts 1.5+2.5 HD ReMIX")
}

fn apply_install_path(ui: &AppWindow, path: String) {
    ui.set_install_path(path.clone().into());
    set_install_indicators(ui, &path);
}

#[tokio::main]
async fn main() -> Result<(), slint::PlatformError> {
    let ui = AppWindow::new()?;
    let ui_handle = ui.as_weak();
    let cancel_flag = Arc::new(AtomicBool::new(false));

    if let Some(u) = ui_handle.upgrade() {
        populate_name_sizes(&u);
        apply_install_path(&u, steam_default_path());
    }

    ui.on_request_directory({
        let ui_handle = ui_handle.clone();
        move || {
            if let Some(folder) = FileDialog::new().pick_folder() {
                if let Some(ui) = ui_handle.upgrade() {
                    apply_install_path(&ui, folder.to_string_lossy().to_string());
                }
            }
        }
    });

    ui.on_platform_changed({
        let ui_handle = ui_handle.clone();
        move |is_steam_now| {
            if let Some(ui) = ui_handle.upgrade() {
                let new_path = if is_steam_now { steam_default_path() } else { epic_default_path() };
                apply_install_path(&ui, new_path);
            }
        }
    });

    ui.on_enforce_mare_deps({
        let ui_handle = ui_handle.clone();
        move |_which| {
            let ui = ui_handle.unwrap();
            let is_install = ui.get_is_install();
            if !is_install { return; }
            if !sel_days && !sel_coded { return; }
            if !ui.get_sel_mare() { ui.set_sel_mare(true); }
            if !ui.get_sel_base() { ui.set_sel_base(true); }
        }
    });

    ui.on_request_cancel({
        let cancel_flag = cancel_flag.clone();
        move || {
            cancel_flag.store(true, Ordering::Relaxed);
        }
    });

    ui.on_request_procedure({
        let ui_handle = ui_handle.clone();
        let closure_cancel_flag = cancel_flag.clone();
        move || {
            let ui = ui_handle.unwrap();
            closure_cancel_flag.store(false, Ordering::Relaxed);
            let is_install = ui.get_is_install();
            let is_steam = ui.get_is_steam();
            let path = ui.get_install_path().to_string();
            let platform_name = if is_steam { "Steam" } else { "Epic Games Store" };

            let sel_days = ui.get_sel_days();
            let sel_coded = ui.get_sel_coded();
            let sel_mare = ui.get_sel_mare();
            let sel_base = ui.get_sel_base();

            if is_install && !sel_mare {
                let proceed = MessageDialog::new()
                    .set_level(MessageLevel::Warning)
                    .set_title("About your selection")
                    .set_description(
                        "You have not selected the Mare component.\nYou will not be able to use the official game launcher, launch the game from Steam/Heroic without extra launch options or alternative launchers or watch the movies.\n\nLaunching the games individually will work. Proceed with the installation?"
                    )
                    .set_buttons(MessageButtons::YesNo)
                    .show();

                if proceed != MessageDialogResult::Yes {
                    ui.set_display_info("Operation cancelled.".into());
                    return;
                }
            }

            if is_steam {
                let want_launchers = MessageDialog::new()
                    .set_level(MessageLevel::Info)
                    .set_title("Custom launcher")
                    .set_description(
                        "Would you like to download my custom launchers to replace the official one?\n\nPressing 'Yes' will redirect to a GitHub page with instructions."
                    )
                    .set_buttons(MessageButtons::YesNo)
                    .show();

                if want_launchers == MessageDialogResult::Yes {
                    let _ = open::that(LAUNCHERS_URL);
                }
            }

            if !is_steam {
                let egsheroic = MessageDialog::new()
                    .set_level(MessageLevel::Warning)
                    .set_title("About Epic Games Store...")
                    .set_description(
                        "When installing via Epic Games Store, you will not be able to launch the games via the official Epic Games Launcher due to limitations I couldn't work around. Therefore, you must use Heroic.\n\nInstall Heroic if you didn't already, or press 'Cancel' to cancel installation."
                    )
                    .set_buttons(MessageButtons::OkCancel)
                    .show();

                if egsheroic == MessageDialogResult::Cancel {
                    ui.set_display_info("Operation cancelled.".into());
                    return;
                }
            }

            let selected_names: Vec<String> = GAMES.iter()
                .filter(|g| get_sel(&ui, g.sel_property))
                .map(|g| g.display_name_short.to_string())
                .collect();

            let total_size: f32 = GAMES.iter()
                .filter(|g| get_sel(&ui, g.sel_property))
                .map(|g| g.size_gib)
                .sum();

            let action_text = if is_install { "INSTALL" } else { "UNINSTALL" };
            let confirmation_msg = format!(
                "You have chosen to {action_text} the following components on {platform_name}:\n\n- {}\n\nTotal download size: {total_size:.2} GiB\n\nTarget Directory:\n{path}\n\nAre you sure? You will be prompted to log into {platform_name} next.",
                selected_names.join("\n- ")
            );

            let confirm = MessageDialog::new()
                .set_level(MessageLevel::Info)
                .set_title("Confirm procedure")
                .set_description(&confirmation_msg)
                .set_buttons(MessageButtons::YesNo)
                .show();

            if confirm != MessageDialogResult::Yes {
                ui.set_display_info("Operation cancelled.".into());
                return;
            }

            if is_install {
                let required_bytes = (total_size * 1073741824.0) as u64;
                let free_bytes = available_space(Path::new(&path)).unwrap_or(u64::MAX);

                if free_bytes < required_bytes {
                    let free_gib = free_bytes as f32 / 1073741824.0;
                    MessageDialog::new()
                        .set_level(MessageLevel::Error)
                        .set_title("Insufficient storage")
                        .set_description(&format!(
                            "There is not enough free space on the selected drive.\n\nRequired: {:.2} GiB\nAvailable: {:.2} GiB",
                            total_size, free_gib
                        ))
                        .show();
                    ui.set_display_info("Operation cancelled due to insufficient storage.".into());
                    return;
                }
            }

            if let Err(e) = permissions::ensure_writable(Path::new(&path)) {
                MessageDialog::new()
                    .set_level(MessageLevel::Error)
                    .set_title("Permission error. Ensure the selected folder has write permissions for the current user.")
                    .set_description(&e)
                    .show();
                ui.set_display_info("Operation cancelled.".into());
                return;
            }

            ui.set_is_busy(true);

            let weak_ui = ui_handle.clone();

            if !is_install {
                let pattern_groups: Vec<Vec<String>> = if is_steam {
                    let mut depot1_regex: Vec<String> = Vec::new();
                    let mut depot2_regex: Vec<String> = Vec::new();

                    if sel_base {
                        depot1_regex.push(r"regex:^(Image/SettingMenu\.(hed|pkg)|steam_api64\.dll|WaitTitleProject\.exe)$".into());
                    }
                    if sel_mare {
                        depot1_regex.push(r"regex:^(Image/Mare\.(hed|pkg)|KINGDOM HEARTS HD 1\.5\+2\.5 (Launcher|ReMIX)\.exe)$".into());
                    }
                    for g in GAMES.iter().filter(|g| g.id != "base" && g.id != "mare") {
                        if get_sel(&ui, g.sel_property) {
                            if let Some(r) = g.depot1_regex { depot1_regex.push(r.to_string()); }
                            if let Some(r) = g.depot2_regex { depot2_regex.push(r.to_string()); }
                        }
                    }
                    vec![depot1_regex, depot2_regex]
                } else {
                    let mut epic_patterns: Vec<String> = Vec::new();
                    for g in GAMES.iter() {
                        if get_sel(&ui, g.sel_property) {
                            epic_patterns.extend(g.epic_regex.iter().map(|s| s.to_string()));
                        }
                    }
                    vec![epic_patterns]
                };

                tokio::spawn(async move {
                    run_uninstall(pattern_groups, path, weak_ui).await;
                });
                return;
            }

            if is_steam {
                let mut depot1_regex: Vec<String> = Vec::new();
                let mut depot2_regex: Vec<String> = Vec::new();

                if sel_base {
                    depot1_regex.push(r"regex:^(Image/SettingMenu\.(hed|pkg)|steam_api64\.dll|WaitTitleProject\.exe)$".into());
                }
                if sel_mare {
                    depot1_regex.push(r"regex:^(Image/Mare\.(hed|pkg)|KINGDOM HEARTS HD 1\.5\+2\.5 (Launcher|ReMIX)\.exe)$".into());
                }
                for g in GAMES.iter().filter(|g| g.id != "base" && g.id != "mare") {
                    if get_sel(&ui, g.sel_property) {
                        if let Some(r) = g.depot1_regex { depot1_regex.push(r.to_string()); }
                        if let Some(r) = g.depot2_regex { depot2_regex.push(r.to_string()); }
                    }
                }

                let depots = vec![
                    (DepotId(2552433), depot1_regex),
                    (DepotId(2552435), depot2_regex),
                ];

                ui.set_login_title("Steam Login".into());
                ui.set_login_busy(false);
                ui.set_login_show_guard(false);
                ui.set_login_show_qr(false);
                ui.set_login_show_mobile_confirm(false);
                ui.set_login_status("Enter your Steam credentials or use QR login.".into());
                ui.set_login_visible(true);
                ui.set_display_info("Waiting for Steam login...".into());

                let (auth_tx, auth_rx) = mpsc::unbounded_channel::<AuthInput>();
                let auth_tx = Arc::new(auth_tx);

                {
                    let tx = auth_tx.clone();
                    let weak = ui_handle.clone();
                    ui.on_login_submit_credentials(move |username, password| {
                        if let Some(ui) = weak.upgrade() {
                            ui.set_login_busy(true);
                            ui.set_login_status("Signing in...".into());
                        }
                        let _ = tx.send(AuthInput::Credentials { username: username.to_string(), password: password.to_string() });
                    });
                }
                {
                    let tx = auth_tx.clone();
                    let weak = ui_handle.clone();
                    ui.on_login_submit_guard(move |code| {
                        if let Some(ui) = weak.upgrade() {
                            ui.set_login_busy(true);
                            ui.set_login_status("Verifying...".into());
                        }
                        let _ = tx.send(AuthInput::GuardCode(code.to_string()));
                    });
                }
                {
                    let tx = auth_tx.clone();
                    let weak = ui_handle.clone();
                    ui.on_login_request_qr(move || {
                        if let Some(ui) = weak.upgrade() {
                            ui.set_login_busy(true);
                            ui.set_login_status("Requesting QR code...".into());
                        }
                        let _ = tx.send(AuthInput::RequestQr);
                    });
                }

                let task_cancel_flag = closure_cancel_flag.clone();
                tokio::spawn(run_install_flow(
                    auth_rx,
                    weak_ui,
                    path,
                    depots,
                    task_cancel_flag
                ));
            } else {
                let mut epic_patterns: Vec<String> = Vec::new();
                for g in GAMES.iter() {
                    if get_sel(&ui, g.sel_property) {
                        epic_patterns.extend(g.epic_regex.iter().map(|s| s.to_string()));
                    }
                }

                ui.set_login_title("Epic Games Login".into());
                ui.set_login_busy(false);
                ui.set_login_show_guard(true);
                ui.set_login_show_qr(false);
                ui.set_login_show_mobile_confirm(false);
                ui.set_login_code_label("Epic Authorization Code".into());
                ui.set_login_code_placeholder("Paste the 'authorizationCode' value from the page that opens".into());
                ui.set_login_status("A browser window will open for Epic login.".into());
                ui.set_login_visible(true);
                ui.set_display_info("Waiting for Epic Games login...".into());

                let (epic_tx, epic_rx) = mpsc::unbounded_channel::<EpicAuthInput>();
                let epic_tx = Arc::new(epic_tx);
                {
                    let tx = epic_tx.clone();
                    let weak = ui_handle.clone();
                    ui.on_login_submit_guard(move |code| {
                        if let Some(ui) = weak.upgrade() {
                            ui.set_login_busy(true);
                            ui.set_login_status("Submitting code...".into());
                        }
                        let _ = tx.send(EpicAuthInput::AuthCode(code.to_string()));
                    });
                }

                let task_cancel_flag = closure_cancel_flag.clone();
                tokio::spawn(epic::auth::run_install_flow(
                    epic_rx,
                    weak_ui,
                    path,
                    epic_patterns,
                    task_cancel_flag,
                ));
            }
        }
    });

    ui.on_login_cancel({
        let ui_handle = ui_handle.clone();
        move || {
            if let Some(ui) = ui_handle.upgrade() {
                ui.set_login_visible(false);
                ui.set_login_busy(false);
                ui.set_is_busy(false);
                ui.set_display_info("Login cancelled.".into());

                ui.on_login_submit_credentials(|_, _| {});
                ui.on_login_submit_guard(|_| {});
                ui.on_login_request_qr(|| {});
            }
        }
    });

    ui.on_login_submit_credentials(|_, _| {});
    ui.on_login_submit_guard(|_| {});
    ui.on_login_request_qr(|| {});

    ui.run()
}