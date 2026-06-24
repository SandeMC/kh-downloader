use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use tokio::sync::mpsc;

use steamroom::client::SteamClient;
use steamroom::client::LoggedIn;

use steamroom_client::login::{LoginBuilder, GuardType, LoginError};
use steamroom_client::login::CredentialsLoginFlow;

use crate::AppWindow;
use crate::ui::set_login_ui;

pub enum AuthInput {
    Credentials { username: String, password: String },
    GuardCode(String),
    RequestQr,
}

pub async fn run_install_flow(
    mut auth_rx: mpsc::UnboundedReceiver<AuthInput>,
    weak_ui: slint::Weak<AppWindow>,
    path: String,
    depots: Vec<(steamroom::depot::DepotId, Vec<String>)>,
    cancel_flag: Arc<AtomicBool>,
) {
    let logged_in = match drive_login(&mut auth_rx, &weak_ui).await {
        Ok(c) => c,
        Err(e) => {
            let msg = format!("Login failed: {e}");
            let weak = weak_ui.clone();
            let _ = slint::invoke_from_event_loop(move || {
                if let Some(ui) = weak.upgrade() {
                    ui.set_login_visible(false);
                    ui.set_is_busy(false);
                }
                rfd::MessageDialog::new().set_level(rfd::MessageLevel::Error)
                    .set_title("Login Error").set_description(&msg).show();
            });
            return;
        }
    };

    let weak = weak_ui.clone();
    let _ = slint::invoke_from_event_loop(move || {
        if let Some(ui) = weak.upgrade() { ui.set_login_visible(false); }
    });

    let app_id = 2552430u32;
    crate::steam::install::run_install(&logged_in, app_id, depots, &path, &weak_ui, cancel_flag).await;
}

pub async fn drive_login(
    auth_rx: &mut mpsc::UnboundedReceiver<AuthInput>,
    weak_ui: &slint::Weak<AppWindow>,
) -> Result<SteamClient<LoggedIn>, Box<dyn std::error::Error + Send + Sync>> {
    loop {
        let input = auth_rx.recv().await.ok_or("Login dialog closed")?;
        match input {
            AuthInput::Credentials { username, password } => {
                return credentials_login(username, password, auth_rx, weak_ui).await;
            }
            AuthInput::RequestQr => {
                return qr_login(auth_rx, weak_ui).await;
            }
            AuthInput::GuardCode(_) => {}
        }
    }
}

async fn credentials_login(
    username: String,
    password: String,
    auth_rx: &mut mpsc::UnboundedReceiver<AuthInput>,
    weak_ui: &slint::Weak<AppWindow>,
) -> Result<SteamClient<LoggedIn>, Box<dyn std::error::Error + Send + Sync>> {
    set_login_ui(weak_ui, "Connecting to Steam...", false, false, false, true, None);

    let flow = LoginBuilder::new()
        .device_name("KH Downloader")
        .with_credentials(username, password)
        .begin()
        .await?;

    match flow {
        CredentialsLoginFlow::Approved(auth) => {
            set_login_ui(weak_ui, "Logging in...", false, false, false, true, None);
            Ok(auth.finish().await?)
        }
        CredentialsLoginFlow::NeedsGuardCode(mut challenge) => {
            let is_email = challenge.allowed_kinds().iter().any(|k| matches!(k, GuardType::EmailCode));
            let status = if is_email {
                "Enter the Steam Guard code sent to your email."
            } else {
                "Enter the code from the Steam Guard on the mobile Steam app."
            };
            set_login_ui(weak_ui, status, true, false, false, false, None);

            loop {
                let input = auth_rx.recv().await.ok_or("Login dialog closed")?;
                if let AuthInput::GuardCode(code) = input {
                    set_login_ui(weak_ui, "Submitting code...", true, false, false, true, None);
                    let kind = if is_email { GuardType::EmailCode } else { GuardType::DeviceCode };
                    match challenge.submit_code(&code, kind).await {
                        Ok(auth) => {
                            set_login_ui(weak_ui, "Logging in...", false, false, false, true, None);
                            return Ok(auth.finish().await?);
                        }
                        Err((returned_challenge, LoginError::InvalidGuardCode)) => {
                            challenge = returned_challenge;
                            set_login_ui(weak_ui, "Incorrect code. Try again.", true, false, false, false, None);
                        }
                        Err((_, e)) => return Err(e.into()),
                    }
                }
            }
        }
        CredentialsLoginFlow::NeedsMobileConfirm(challenge) => {
            set_login_ui(weak_ui, "Awaiting approval...", false, false, true, true, None);
            let auth = challenge.wait_for_confirmation().await?;
            set_login_ui(weak_ui, "Logging in...", false, false, false, true, None);
            Ok(auth.finish().await?)
        }
        _ => Err("Unexpected login flow state encountered.".into()),
    }
}

async fn qr_login(
    _auth_rx: &mut mpsc::UnboundedReceiver<AuthInput>,
    weak_ui: &slint::Weak<AppWindow>,
) -> Result<SteamClient<LoggedIn>, Box<dyn std::error::Error + Send + Sync>> {
    set_login_ui(weak_ui, "Connecting to Steam...", false, false, false, true, None);

    let flow = LoginBuilder::new()
        .device_name("Kingdom Hearts Downloader")
        .with_qr()
        .begin()
        .await?;

    set_login_ui(
        weak_ui,
        "Scan the QR code in the mobile app to continue.",
        false, true, false, false,
        Some(flow.challenge_url()),
    );

    let auth = flow.wait_for_scan().await?;
    set_login_ui(weak_ui, "Logging in...", false, false, false, true, None);
    Ok(auth.finish().await?)
}