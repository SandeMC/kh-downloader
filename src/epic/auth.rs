use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use tokio::sync::mpsc;

use egs_api::EpicGames;

use crate::ui::set_login_ui;
use crate::AppWindow;

// Epic's public launcher client ID - this is the same one used by every third-party Epic client (Legendary, Heroic, etc.), not a secret.
const EPIC_CLIENT_ID: &str = "34a02cf8f4414e29b15921876da36f9a";

pub enum EpicAuthInput {
    AuthCode(String),
}

fn authorization_url() -> String {
    format!(
        "https://www.epicgames.com/id/login?redirectUrl=https%3A%2F%2Fwww.epicgames.com%2Fid%2Fapi%2Fredirect%3FclientId%3D{EPIC_CLIENT_ID}%26responseType%3Dcode"
    )
}

pub async fn run_install_flow(
    mut auth_rx: mpsc::UnboundedReceiver<EpicAuthInput>,
    weak_ui: slint::Weak<AppWindow>,
    path: String,
    patterns: Vec<String>,
    cancel_flag: Arc<AtomicBool>,
) {
    let egs = match drive_epic_login(&mut auth_rx, &weak_ui).await {
        Ok(c) => c,
        Err(e) => {
            let msg = format!("Epic login failed: {}", e);
            let weak = weak_ui.clone();
            let _ = slint::invoke_from_event_loop(move || {
                if let Some(ui) = weak.upgrade() {
                    ui.set_login_visible(false);
                    ui.set_is_busy(false);
                }
                rfd::MessageDialog::new()
                    .set_level(rfd::MessageLevel::Error)
                    .set_title("Epic Login Error")
                    .set_description(&msg)
                    .show();
            });
            return;
        }
    };

    let weak = weak_ui.clone();
    let _ = slint::invoke_from_event_loop(move || {
        if let Some(ui) = weak.upgrade() {
            ui.set_login_visible(false);
        }
    });

    crate::epic::install::run_install(egs, patterns, path, weak_ui, cancel_flag).await;
}

/// login flow: opens a browser to Epic's login page, waits for the user to paste back the `authorizationCode` value from the
/// JSON response, then exchanges it for a session.

pub async fn drive_epic_login(
    auth_rx: &mut mpsc::UnboundedReceiver<EpicAuthInput>,
    weak_ui: &slint::Weak<AppWindow>,
) -> Result<EpicGames, Box<dyn std::error::Error + Send + Sync>> {
    let url = authorization_url();
    if open::that(&url).is_err() {
        return Err(format!("Could not open a browser. Go to this URL manually:\n{}", url).into());
    }

    set_login_ui(
        weak_ui,
        "A browser window opened. Log in, then copy the 'authorizationCode' \
         value from the JSON response and paste it below.",
        true,
        false,
        false,
        false,
        None,
    );

    let code = loop {
        match auth_rx.recv().await.ok_or("Login dialog closed")? {
            EpicAuthInput::AuthCode(raw) => {
                let trimmed = raw.trim().trim_matches('"').to_string();
                if !trimmed.is_empty() {
                    break trimmed;
                }
            }
        }
    };

    set_login_ui(weak_ui, "Logging in...", false, false, false, true, None);

    let mut egs = EpicGames::new();
    if !egs.auth_code(None, Some(code)).await {
        return Err("Epic rejected the authorization code. It may have expired - codes are single-use and short-lived, try again from a fresh login.".into());
    }

    egs.login().await;

    if !egs.is_logged_in() {
        return Err("Epic session could not be established after login.".into());
    }

    Ok(egs)
}