use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use steamroom::cdn::CdnClient;
use steamroom::cdn::pool::CdnServerPool;
use steamroom::depot::{AppId, CellId, DepotId, DepotKey, ManifestId};
use steamroom::depot::manifest::DepotManifest;
use steamroom::client::{SteamClient, LoggedIn};

use steamroom_client::download::{DepotJob, FileFilter, CdnChunkFetcher};
use steamroom_client::event::DownloadEvent;

use tokio::sync::mpsc;

use crate::AppWindow;
use crate::ui::{update_status, show_error, set_install_indicators};

pub async fn fetch_depot_context(
    client: &SteamClient<LoggedIn>,
    app_id: u32,
    depot_id: DepotId,
) -> Result<(DepotManifest, DepotKey, CdnChunkFetcher), Box<dyn std::error::Error + Send + Sync>> {

    let manifest_id = match depot_id.0 {
        2552433 => ManifestId(2946731077053901934),
        2552435 => ManifestId(3908821002986173448),
        _ => return Err(format!("No hardcoded manifest ID for depot {}", depot_id.0).into()),
    };

    let depot_key = client.get_depot_decryption_key(depot_id, AppId(app_id)).await?;
    let request_code = client
        .get_manifest_request_code(AppId(app_id), depot_id, manifest_id, Some("public"), None)
        .await?
        .unwrap_or(0);

    let cdn_servers = client.get_cdn_servers(CellId(0), None).await?;
    let pool = CdnServerPool::new(cdn_servers);
    let cdn_client = CdnClient::new()?;

    let mut manifest_bytes = None;
    let mut last_err = String::new();

    for _ in 0..5 {
        let (server, _) = pool.pick();
        match cdn_client.download_manifest(server, depot_id, manifest_id, request_code, None).await {
            Ok(bytes) => {
                pool.report_success(server);
                manifest_bytes = Some(bytes);
                break;
            }
            Err(e) => {
                pool.report_failure(server, None);
                last_err = e.to_string();
                tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            }
        }
    }

    let manifest_bytes = manifest_bytes.ok_or_else(|| {
        format!("Manifest download failed after 5 retries. Last error: {last_err}")
    })?;

    let payload = if manifest_bytes.len() > 2 && manifest_bytes[0] == b'P' && manifest_bytes[1] == b'K' {
        let cursor = std::io::Cursor::new(&manifest_bytes);
        let mut archive = zip::ZipArchive::new(cursor)?;
        let mut file = archive.by_index(0)?;
        let mut decompressed_bytes = Vec::new();
        std::io::Read::read_to_end(&mut file, &mut decompressed_bytes)?;
        decompressed_bytes
    } else {
        manifest_bytes.to_vec()
    };

    let mut manifest = DepotManifest::parse(&payload)?;
    if manifest.filenames_encrypted {
        manifest.decrypt_filenames(&depot_key)?;
    }

    Ok((manifest, depot_key, CdnChunkFetcher::new(cdn_client, pool, None)))
}

pub async fn run_install(
    client: &SteamClient<LoggedIn>,
    app_id: u32,
    depots: Vec<(DepotId, Vec<String>)>,
    path: &str,
    weak_ui: &slint::Weak<AppWindow>,
    cancel_flag: Arc<AtomicBool>,
) {
    let mut had_errors = false;
    let mut contexts = Vec::new();

    for (depot_id, regex_list) in &depots {
        if regex_list.is_empty() { continue; }

        update_status(weak_ui, &format!("Fetching metadata for depot {}...", depot_id.0));

        match fetch_depot_context(client, app_id, *depot_id).await {
            Ok(ctx) => contexts.push((*depot_id, regex_list.clone(), ctx)),
            Err(e) => {
                show_error(&format!("Failed to fetch depot {}: {e}", depot_id.0));
                had_errors = true;
            }
        };
    }

    if had_errors {
        let weak = weak_ui.clone();
        let _ = slint::invoke_from_event_loop(move || {
            if let Some(ui) = weak.upgrade() {
                ui.set_display_info("Setup failed. Check errors.".into());
                ui.set_is_busy(false);
            }
        });
        return;
    }

    for (depot_id, regex_list, (manifest, depot_key, fetcher)) in contexts {
        if cancel_flag.load(Ordering::Relaxed) {
            had_errors = true;
            show_error("Installation cancelled by user.");
            break;
        }

        let filter = match FileFilter::from_filelist(&regex_list) {
            Ok(f) => f,
            Err(e) => {
                show_error(&format!("Bad regex for depot {}: {e}", depot_id.0));
                had_errors = true;
                continue;
            }
        };

        let installed_filter = match FileFilter::from_filelist(&regex_list) {
            Ok(f) => f,
            Err(e) => {
                show_error(&format!("Bad regex for depot {}: {e}", depot_id.0));
                had_errors = true;
                continue;
            }
        };

        let (event_tx, mut event_rx) = mpsc::unbounded_channel();

        let job = match DepotJob::builder()
            .depot_id(depot_id)
            .depot_key(depot_key)
            .install_dir(path.into())
            .verify(true)
            .file_filter(filter)
            .event_sender(event_tx)
            .build()
        {
            Ok(j) => j,
            Err(e) => {
                show_error(&format!("DepotJob build failed: {e}"));
                had_errors = true;
                continue;
            }
        };

        let download_task = tokio::spawn(async move {
            job.download(&manifest, Arc::new(fetcher)).await
        });

        let mut current_stage = String::from("Initialization");
        let mut depot_total_bytes: u64 = 0;
        let mut depot_bytes_so_far: u64 = 0;

        while let Some(event) = event_rx.recv().await {
            if cancel_flag.load(Ordering::Relaxed) {
                download_task.abort();
                had_errors = true;
                break;
            }
            match event {
                DownloadEvent::DownloadStarted { total_bytes, total_files } => {
                    depot_total_bytes = total_bytes;
                    depot_bytes_so_far = 0;
                    current_stage = format!(
                        "Depot {}: {total_files} files, {:.2} GiB",
                        depot_id.0, total_bytes as f64 / 1024.0 / 1024.0 / 1024.0
                    );
                    update_status(weak_ui, &current_stage);
                }
                DownloadEvent::ChunkCompleted { bytes } => {
                    depot_bytes_so_far += bytes;
                    if depot_total_bytes > 0 {
                        let progress = (depot_bytes_so_far as f64 / depot_total_bytes as f64) as f32;
                        let weak = weak_ui.clone();
                        let _ = slint::invoke_from_event_loop(move || {
                            if let Some(ui) = weak.upgrade() { ui.set_download_progress(progress); }
                        });
                    }
                }
                DownloadEvent::FileStarted { filename } => {
                    current_stage = format!("Downloading: {filename}");
                    update_status(weak_ui, &current_stage);
                }
                DownloadEvent::FileCompleted { filename } => {
                    current_stage = format!("Downloaded: {filename}");
                    update_status(weak_ui, &current_stage);
                }
                DownloadEvent::FileSkipped { filename } => {
                    if installed_filter.matches(&filename) {
                        current_stage = format!("Already installed: {filename}");
                        update_status(weak_ui, &current_stage);
                    }
                }
                DownloadEvent::FileRemoved { filename } => {
                    current_stage = format!("Removing conflicting file: {filename}");
                    update_status(weak_ui, &current_stage);
                }
                DownloadEvent::ChunkFailed { error } => {
                    update_status(weak_ui, &format!("Chunk retry: {error}"));
                }
                _ => {}
            }
        }

        match download_task.await {
            Ok(Ok(_)) => {}
            Ok(Err(e)) => {
                had_errors = true;
                let mut msg = format!("Download error in depot {}:\n{e}", depot_id.0);

                let is_denied = e.downcast_ref::<std::io::Error>()
                    .map(|io_err| io_err.kind() == std::io::ErrorKind::PermissionDenied)
                    .unwrap_or(false);

                if is_denied || msg.contains("os error 5") || msg.contains("Access is denied") {
                    msg.push_str("\n\nWindows is likely blocking the app from writing to this file. Try running the downloader as Administrator, or choose a different install location.");
                }

                show_error(&msg);
            }
            Err(e) if e.is_cancelled() => {
                had_errors = true;
                show_error("Installation cancelled by user.");
                break;
            }
            Err(e) => {
                had_errors = true;
                show_error(&format!("Critical error at depot {}: {e}", depot_id.0));
            }
        }
    }

    if !had_errors {
        write_app_manifest(path);
        let _ = slint::invoke_from_event_loop(|| {
            rfd::MessageDialog::new().set_level(rfd::MessageLevel::Info)
                .set_title("Installation complete")
                .set_description("All components downloaded.\n\nThe game should now appear in your Steam library after restarting Steam fully.\nRefer to instructions on the GitHub repository if it didn't.")
                .show();
        });
    }

    let weak = weak_ui.clone();
    let path_str = path.to_string();
    let _ = slint::invoke_from_event_loop(move || {
        if let Some(ui) = weak.upgrade() {
            ui.set_display_info(if had_errors { "Finished with errors." } else { "All operations complete!" }.into());
            if !had_errors { ui.set_download_progress(1.0); }
            ui.set_is_busy(false);
            set_install_indicators(&ui, &path_str);
        }
    });
}

pub fn write_app_manifest(install_path: &str) {
    let install_dir = std::path::Path::new(install_path);
    if let (Some(install_dir_name), Some(common_dir)) = (install_dir.file_name(), install_dir.parent()) {
        if let Some(steamapps_dir) = common_dir.parent() {
            let install_dir_name = install_dir_name.to_string_lossy();
            let timestamp_secs = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs();

            let acf_content = format!(
                "\"AppState\"\n\
                {{\n\
                \t\"appid\"\t\t\"2552430\"\n\
                \t\"Universe\"\t\t\"1\"\n\
                \t\"name\"\t\t\"KINGDOM HEARTS -HD 1.5+2.5 ReMIX-\"\n\
                \t\"StateFlags\"\t\t\"4\"\n\
                \t\"installdir\"\t\t\"{install_dir_name}\"\n\
                \t\"LastUpdated\"\t\t\"{timestamp_secs}\"\n\
                \t\"UpdateResult\"\t\t\"0\"\n\
                \t\"SizeOnDisk\"\t\t\"0\"\n\
                \t\"buildid\"\t\t\"0\"\n\
                \t\"AutoUpdateBehavior\"\t\t\"0\"\n\
                \t\"InstalledDepots\"\n\
                \t{{\n\
                \t\t\"2552433\"\n\
                \t\t{{\n\
                \t\t\t\"manifest\"\t\t\"2946731077053901934\"\n\
                \t\t\t\"size\"\t\t\"0\"\n\
                \t\t}}\n\
                \t\t\"2552435\"\n\
                \t\t{{\n\
                \t\t\t\"manifest\"\t\t\"3908821002986173448\"\n\
                \t\t\t\"size\"\t\t\"0\"\n\
                \t\t}}\n\
                \t}}\n\
                }}",
            );
            let _ = std::fs::write(steamapps_dir.join("appmanifest_2552430.acf"), acf_content);
        }
    }
}