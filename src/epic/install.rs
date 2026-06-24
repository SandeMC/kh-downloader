use std::collections::HashMap;
use std::fs::File;
use std::io::Write as _;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;

use egs_api::api::types::chunk::Chunk;
use egs_api::api::types::download_manifest::{DownloadManifest, FileChunkPart, FileManifestList};
use egs_api::EpicGames;

use sha1::{Digest, Sha1};
use steamroom_client::download::FileFilter;
use tokio::sync::Semaphore;

use crate::ui::{set_install_indicators, show_error, update_status};
use crate::AppWindow;

// IDs for KINGDOM HEARTS HD 1.5+2.5 ReMIX on Epic
const NAMESPACE: &str = "4158b699dd70447a981fee752d970a3e";
const ITEM_ID: &str = "5aac304f0e8948268ddfd404334dbdc7";
const APP_NAME: &str = "68c214c58f694ae88c2dab6f209b43e4";

const CONCURRENT_CHUNK_DOWNLOADS: usize = 8;

/// logic flow: fetches the manifest, downloads every chunk referenced by the selected files,
/// decompresses each via egs-api's own `Chunk::from_vec`, then reassembles + SHA1-verifies every file.
///
/// egs-api only parses manifests, it doesn't fetch or decode chunks.
/// Chunk download is based on AchetaGames/Epic-Asset-Manager code, adapted by AI for this project.
/// License: https://github.com/AchetaGames/Epic-Asset-Manager/blob/main/LICENSE (MIT License (c) 2021 Acheta Games)
pub async fn run_install(
    mut egs: EpicGames,
    patterns: Vec<String>,
    path: String,
    weak_ui: slint::Weak<AppWindow>,
    cancel_flag: Arc<AtomicBool>,
) {
    update_status(&weak_ui, "Fetching Epic asset manifest...");

    let manifest = match egs
        .asset_manifest(
            None,
            None,
            Some(NAMESPACE.to_string()),
            Some(ITEM_ID.to_string()),
            Some(APP_NAME.to_string()),
        )
        .await
    {
        Some(m) => m,
        None => {
            show_error(
                "Epic returned no asset manifest for this namespace/item/app. \
                 Double-check the constants at the top of epic/install.rs.",
            );
            finish(&weak_ui, false, &path);
            return;
        }
    };

    update_status(&weak_ui, "Fetching Epic download manifest...");

    let download_manifest: DownloadManifest = match egs.asset_download_manifests(manifest).await.into_iter().next() {
        Some(d) => d,
        None => {
            show_error("No download manifest could be parsed from any CDN mirror.");
            finish(&weak_ui, false, &path);
            return;
        }
    };

    let filter = match FileFilter::from_filelist(&patterns) {
        Ok(f) => f,
        Err(e) => {
            show_error(&format!("Bad regex in Epic file patterns: {}", e));
            finish(&weak_ui, false, &path);
            return;
        }
    };

    let files: Vec<FileManifestList> = download_manifest
        .files()
        .into_values()
        .filter(|f| filter.matches(&f.filename))
        .collect();

    if files.is_empty() {
        show_error("No files in the Epic manifest matched the selected components.");
        finish(&weak_ui, false, &path);
        return;
    }

    // Dedup chunks up front - the same chunk can back parts of multiple files.
    let mut chunk_parts: HashMap<String, FileChunkPart> = HashMap::new();
    for f in &files {
        for part in &f.file_chunk_parts {
            chunk_parts.entry(part.guid.clone()).or_insert_with(|| part.clone());
        }
    }

    let total_download_bytes: u64 = chunk_parts
        .keys()
        .filter_map(|guid| download_manifest.chunk_filesize_list.get(guid))
        .map(|v| *v as u64)
        .sum();

    let temp_dir = Path::new(&path).join(".epic_chunks_tmp");
    if let Err(e) = std::fs::create_dir_all(&temp_dir) {
        show_error(&format!("Could not create temp directory {:?}: {}", temp_dir, e));
        finish(&weak_ui, false, &path);
        return;
    }

    update_status(
        &weak_ui,
        &format!(
            "Downloading {} chunks ({:.2} GiB)...",
            chunk_parts.len(),
            total_download_bytes as f64 / 1024.0 / 1024.0 / 1024.0
        ),
    );

    let client = reqwest::Client::new();
    let semaphore = Arc::new(Semaphore::new(CONCURRENT_CHUNK_DOWNLOADS));
    let downloaded_bytes = Arc::new(AtomicU64::new(0));
    let mut had_errors = false;

    // Pre-credit anything already on disk from a previous interrupted run.
    // (the simplest possible resume support - chunk files that already exist are assumed complete and are not re-validated byte-for-byte).
    let mut to_fetch: Vec<(String, FileChunkPart)> = Vec::new();
    for (guid, part) in &chunk_parts {
        let chunk_path = temp_dir.join(format!("{guid}.chunk"));
        if chunk_path.exists() {
            if let Ok(meta) = std::fs::metadata(&chunk_path) {
                downloaded_bytes.fetch_add(meta.len(), Ordering::Relaxed);
            }
        } else if part.link.is_some() {
            to_fetch.push((guid.clone(), part.clone()));
        } else {
            had_errors = true;
            show_error(&format!(
                "Chunk {} has no resolvable CDN link (missing from chunk_hash_list/data_group_list in the manifest).",
                guid
            ));
        }
    }

    let mut tasks = tokio::task::JoinSet::new();
    for (guid, part) in to_fetch {
        let link = part.link.expect("filtered above");
        let chunk_path = temp_dir.join(format!("{guid}.chunk"));
        let permit = semaphore.clone();
        let client = client.clone();
        let weak = weak_ui.clone();
        let downloaded_bytes = downloaded_bytes.clone();

        tasks.spawn(async move {
            let _permit = permit.acquire_owned().await;
            let result = download_chunk(&client, &link, &chunk_path).await;
            if let Ok(size) = &result {
                let total_downloaded = downloaded_bytes.fetch_add(*size, Ordering::Relaxed) + size;
                if total_download_bytes > 0 {
                    let progress = (total_downloaded as f64 / total_download_bytes as f64) as f32;
                    let progress_weak = weak.clone();
                    let _ = slint::invoke_from_event_loop(move || {
                        if let Some(ui) = progress_weak.upgrade() {
                            ui.set_download_progress(progress);
                        }
                    });
                }
            }
            (guid, result)
        });
    }

    while let Some(res) = tasks.join_next().await {
        if cancel_flag.load(Ordering::Relaxed) {
            tasks.abort_all();
            show_error("Installation cancelled by user.");
            finish(&weak_ui, false, &path);
            return;
        }

        match res {
            Ok((_, Ok(_))) => {}
            Ok((guid, Err(e))) => {
                had_errors = true;
                show_error(&format!("Failed to download chunk {}: {}", guid, e));
            }
            Err(e) => {
                had_errors = true;
                show_error(&format!("Chunk download task panicked: {}", e));
            }
        }
    }

    if had_errors {
        show_error(
            "Some chunks failed to download. Re-run install to retry - chunks already \
             downloaded are kept in .epic_chunks_tmp inside the install folder.",
        );
        finish(&weak_ui, false, &path);
        return;
    }

    update_status(&weak_ui, "Assembling files...");

    let mut assemble_errors = Vec::new();
    for (i, file) in files.iter().enumerate() {
        if cancel_flag.load(Ordering::Relaxed) {
            show_error("Installation cancelled by user during assembly.");
            finish(&weak_ui, false, &path);
            return;
        }

        update_status(
            &weak_ui,
            &format!("Writing {} ({}/{})", file.filename, i + 1, files.len()),
        );
        let out_path = Path::new(&path).join(&file.filename);
        if let Err(e) = assemble_file(&temp_dir, file, &out_path) {
            let mut msg = format!("{}: {}", file.filename, e);
            let is_denied = e
                .downcast_ref::<std::io::Error>()
                .map(|io_err| io_err.kind() == std::io::ErrorKind::PermissionDenied)
                .unwrap_or(false);
            if is_denied {
                msg.push_str(
                    "\n\nHint: Windows is blocking the app from writing to this file. Try \
                     running the downloader as Administrator, or choose a different install \
                     location outside of 'Program Files'.",
                );
            }
            assemble_errors.push(msg);
        }
    }

    let had_errors = had_errors || !assemble_errors.is_empty();

    if !assemble_errors.is_empty() {
        show_error(&format!(
            "{} file(s) failed to assemble or verify:\n\n{}",
            assemble_errors.len(),
            assemble_errors.join("\n")
        ));
    } else {
        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    if !had_errors {
        let _ = slint::invoke_from_event_loop(|| {
            rfd::MessageDialog::new()
                .set_level(rfd::MessageLevel::Info)
                .set_title("Installation Complete")
                .set_description(
                    "All selected components were downloaded and verified.\n\n\
                     To add the game to Heroic: open Heroic → Library → find KINGDOM HEARTS HD 1.5+2.5 ReMIX and click on it \
                      → press 'IMPORT GAME' → select the install folder → press 'IMPORT GAME' again → go back to library. \
                     Heroic's manual import does not verify files, so it will simply accept a partial install without complaint."
                )
                .show();
        });
    }

    finish(&weak_ui, !had_errors, &path);
}

async fn download_chunk(
    client: &reqwest::Client,
    link: &reqwest::Url,
    dest: &Path,
) -> Result<u64, Box<dyn std::error::Error + Send + Sync>> {
    let response = client.get(link.clone()).send().await?.error_for_status()?;
    let bytes = response.bytes().await?;
    std::fs::write(dest, &bytes)?;
    Ok(bytes.len() as u64)
}

fn assemble_file(
    temp_dir: &Path,
    file: &FileManifestList,
    out_path: &Path,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    if let Some(parent) = out_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut output_file = File::create(out_path)?;
    let mut hasher = Sha1::new();

    for part in &file.file_chunk_parts {
        let chunk_path = temp_dir.join(format!("{}.chunk", part.guid));
        let raw_chunk_bytes = std::fs::read(&chunk_path)?;
        let chunk = Chunk::from_vec(raw_chunk_bytes).ok_or("failed to parse downloaded .chunk file")?;

        let start = part.offset as usize;
        let end = (part.offset + part.size) as usize;
        if chunk.data.len() < end {
            return Err(format!(
                "chunk {} ({} bytes) is smaller than the part it's supposed to contain (needs {} bytes)",
                part.guid, chunk.data.len(), end
            )
            .into());
        }

        hasher.update(&chunk.data[start..end]);
        output_file.write_all(&chunk.data[start..end])?;
    }

    let digest = hasher.finalize();
    let computed_hash = digest.iter().fold(String::new(), |mut s, b| {
        use std::fmt::Write;
        let _ = write!(s, "{:02x}", b);
        s
    });

    if computed_hash != file.file_hash {
        return Err(format!("SHA1 mismatch (manifest says {}, got {})", file.file_hash, computed_hash).into());
    }

    Ok(())
}

fn finish(weak_ui: &slint::Weak<AppWindow>, success: bool, path: &str) {
    let weak = weak_ui.clone();
    let path = path.to_string();
    let _ = slint::invoke_from_event_loop(move || {
        if let Some(ui) = weak.upgrade() {
            ui.set_display_info(
                if success { "All Operations Complete!" } else { "Finished with errors." }.into(),
            );
            if success {
                ui.set_download_progress(1.0);
            }
            ui.set_is_busy(false);
            set_install_indicators(&ui, &path);
        }
    });
}