use std::path::Path;

use rfd::{MessageButtons, MessageDialog, MessageDialogResult, MessageLevel};

pub fn ensure_writable(path: &Path) -> Result<(), String> {
    if can_write(path) {
        return Ok(());
    }

    let proceed = MessageDialog::new()
        .set_level(MessageLevel::Warning)
        .set_title("Administrator Permission Needed")
        .set_description(format!(
            "{}\nisn't writable by this app yet.\n\n\
             The downloader will ask Windows for administrator permission just \
             to grant your account write access to this folder - it won't run \
             the rest of the download elevated and won't do anything else.\n\n\
             Ensure that the selected folder is actually a safe folder and not an intentionally protected one.\n\nContinue?",
            path.display()
        ))
        .set_buttons(MessageButtons::YesNo)
        .show();

    if proceed != MessageDialogResult::Yes {
        return Err("Operation cancelled - administrator permission was declined.".into());
    }

    grant_permissions_elevated(path)?;

    if can_write(path) {
        Ok(())
    } else {
        Err(format!(
            "The app still cannot write to {} even after attempting to grant the user the permissions. \
             Try choosing a different install location.",
            path.display()
        ))
    }
}

fn can_write(path: &Path) -> bool {
    if std::fs::create_dir_all(path).is_err() {
        return false;
    }
    let probe = path.join(".kh-downloader_write_test");
    match std::fs::write(&probe, b"probe") {
        Ok(_) => {
            let _ = std::fs::remove_file(&probe);
            true
        }
        Err(_) => false,
    }
}

#[cfg(windows)]
fn grant_permissions_elevated(path: &Path) -> Result<(), String> {
    let windows_username = std::env::var("USERNAME")
        .map_err(|_| "Could not determine the current Windows username.".to_string())?;
    let path_str = path.to_string_lossy().to_string();

    // another failsafe check
    let _ = std::fs::create_dir_all(path);

    let ps_command = format!(
        "Start-Process icacls -ArgumentList '{}','/grant','{}:(OI)(CI)F','/T' -Verb RunAs -Wait",
        path_str.replace('\'', "''"),
        windows_username.replace('\'', "''")
    );

    let exit_status = std::process::Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-Command", &ps_command])
        .status()
        .map_err(|e| format!("Could not launch the elevation prompt: {}", e))?;

    if !exit_status.success() {
        return Err(
            "The elevated permission request didn't complete successfully \
             (it may have been cancelled by user)."
                .into(),
        );
    }

    Ok(())
}

#[cfg(not(windows))]
fn grant_permissions_elevated(path: &Path) -> Result<(), String> {
    Err(format!(
        "{} isn't writable and automatic elevation isn't implemented on Linux for this app. \
         Fix the folder's permissions manually (e.g. `sudo chown -R $USER \"{}\"`) and try again.",
        path.display(),
        path.display()
    ))
}