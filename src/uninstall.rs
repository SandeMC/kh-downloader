use std::path::Path;

use steamroom_client::download::FileFilter;

use crate::games::GAMES;
use crate::AppWindow;
use crate::ui::set_install_indicators;

pub async fn run_uninstall(
    pattern_groups: Vec<Vec<String>>,
    install_path: String,
    weak_ui: slint::Weak<AppWindow>,
) {
    let mut files_removed = 0u64;
    let mut errors: Vec<String> = Vec::new();
    let base_path = Path::new(&install_path);
    let mut all_files = Vec::new();

    if base_path.exists() {
        get_all_files(base_path, base_path, &mut all_files);
    }

    for (i, regex_list) in pattern_groups.iter().enumerate() {
        if regex_list.is_empty() { continue; }

        let filter = match FileFilter::from_filelist(regex_list) {
            Ok(f) => f,
            Err(e) => { errors.push(format!("Bad regex pattern group {}: {}", i, e)); continue; }
        };

        for file in &all_files {
            if filter.matches(file) {
                let abs_path = base_path.join(file);
                if abs_path.exists() {
                    match std::fs::remove_file(&abs_path) {
                        Ok(_) => {
                            files_removed += 1;
                            let display = file.clone();
                            let weak = weak_ui.clone();
                            let _ = slint::invoke_from_event_loop(move || {
                                if let Some(ui) = weak.upgrade() {
                                    ui.set_display_info(format!("Removed: {}", display).into());
                                }
                            });
                        }
                        Err(e) => errors.push(format!("Remove '{}': {}", file, e)),
                    }
                }
            }
        }
    }

    let msg = if errors.is_empty() {
        format!("Uninstall complete. {files_removed} file(s) removed.")
    } else {
        format!("Done with {} error(s). {files_removed} removed.\n\n{}", errors.len(), errors.join("\n"))
    };
    let level = if errors.is_empty() { rfd::MessageLevel::Info } else { rfd::MessageLevel::Warning };
    let weak = weak_ui.clone();
    let path_str = install_path.clone();

    let _ = slint::invoke_from_event_loop(move || {
        if let Some(ui) = weak.upgrade() {
            ui.set_display_info("Uninstall complete.".into());
            ui.set_download_progress(0.0);
            ui.set_is_busy(false);

            set_install_indicators(&ui, &path_str);
            for g in GAMES {
                match g.inst_property {
                    "inst_base" => if !ui.get_inst_base() { ui.set_sel_base(false); },
                    "inst_mare" => if !ui.get_inst_mare() { ui.set_sel_mare(false); },
                    "inst_kh1" => if !ui.get_inst_kh1() { ui.set_sel_kh1(false); },
                    "inst_recom" => if !ui.get_inst_recom() { ui.set_sel_recom(false); },
                    "inst_days" => if !ui.get_inst_days() { ui.set_sel_days(false); },
                    "inst_kh2" => if !ui.get_inst_kh2() { ui.set_sel_kh2(false); },
                    "inst_bbs" => if !ui.get_inst_bbs() { ui.set_sel_bbs(false); },
                    "inst_coded" => if !ui.get_inst_coded() { ui.set_sel_coded(false); },
                    "inst_theater" => if !ui.get_inst_theater() { ui.set_sel_theater(false); },
                    _ => {}
                }
            }
        }
        rfd::MessageDialog::new().set_level(level).set_title("Uninstall Complete").set_description(&msg).show();
    });
}

fn get_all_files(dir: &Path, base: &Path, files: &mut Vec<String>) {
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                get_all_files(&path, base, files);
            } else if let Ok(rel) = path.strip_prefix(base) {
                files.push(rel.to_string_lossy().replace("\\", "/"));
            }
        }
    }
}