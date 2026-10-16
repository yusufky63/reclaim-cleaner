//! Tauri shell: translates UI calls into `reclaim-core` calls and streams job progress.

pub mod helper;
mod state;

use std::collections::HashMap;
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use reclaim_core::apps::{self, InstalledApp};
use reclaim_core::artifacts::{self, Project};
use reclaim_core::clean::{self, CleanReport, DeleteMethod, Failure};
use reclaim_core::duplicates::{self, DupGroup};
use reclaim_core::empty::{self, EmptyFolder};
use reclaim_core::error::ErrorPayload;
use reclaim_core::explorer::{ExtStat, FileView, NodeView, ScanSummary, Tree};
use reclaim_core::history::{self, HistoryEntry};
use reclaim_core::icons;
use reclaim_core::installers::{self, InstallerFile};
use reclaim_core::junk::{self, TargetResult};
use reclaim_core::leftovers::{self, Leftover};
use reclaim_core::organize;
use reclaim_core::paths::{Origin, key};
use reclaim_core::rules::{ArtifactRule, TargetRule, builtin_artifacts, builtin_targets};
use reclaim_core::settings::{Settings, is_portable};
use reclaim_core::startup::{self, StartupItem, StartupSource};
use reclaim_core::system::{
    self, Drive, LaunchOutcome, LibraryFolders, OsInfo, RecycleBinInfo, SystemFile,
};
use reclaim_core::{Error, Job, ProgressSnapshot};
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State};

use crate::state::AppState;

type CmdResult<T> = Result<T, ErrorPayload>;

fn err(e: Error) -> ErrorPayload {
    ErrorPayload::from(&e)
}

const CREATE_NO_WINDOW: u32 = 0x0800_0000;

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct ProgressEvent {
    kind: String,
    #[serde(flatten)]
    snapshot: ProgressSnapshot,
}

/// Runs `work` on a blocking thread as job `kind`, streaming progress events every 150 ms. A
/// new job of the same kind cancels the previous one.
async fn run_job<T, F>(app: &AppHandle, kind: &str, work: F) -> CmdResult<T>
where
    T: Send + 'static,
    F: FnOnce(Arc<Job>) -> Result<T, Error> + Send + 'static,
{
    let state = app.state::<AppState>();
    let job = Arc::new(Job::new());
    if let Some(old) = state.jobs.lock().insert(kind.to_string(), job.clone()) {
        old.cancel();
    }
    let done = Arc::new(AtomicBool::new(false));
    {
        let (app, job, done, kind) = (app.clone(), job.clone(), done.clone(), kind.to_string());
        std::thread::spawn(move || {
            loop {
                let finished = done.load(Ordering::Relaxed);
                let _ = app.emit(
                    "job-progress",
                    ProgressEvent {
                        kind: kind.clone(),
                        snapshot: job.snapshot(),
                    },
                );
                if finished {
                    break;
                }
                std::thread::sleep(Duration::from_millis(150));
            }
        });
    }
    let worker_job = job.clone();
    let res = tauri::async_runtime::spawn_blocking(move || work(worker_job)).await;
    done.store(true, Ordering::Relaxed);
    {
        let mut jobs = state.jobs.lock();
        if jobs.get(kind).is_some_and(|j| Arc::ptr_eq(j, &job)) {
            jobs.remove(kind);
        }
    }
    match res {
        Ok(Ok(v)) => Ok(v),
        Ok(Err(e)) => Err(err(e)),
        Err(e) => Err(err(Error::Other(e.to_string()))),
    }
}

fn record(state: &AppState, kind: &str, method: DeleteMethod, report: &CleanReport) {
    if report.freed + report.recycled + report.files == 0 && report.items.is_empty() {
        return;
    }
    let entry = HistoryEntry::from_report(kind, method, report, system::is_elevated());
    let _ = history::append(&state.data_dir, &entry);
}

fn existing_dirs(roots: &[String]) -> Result<Vec<PathBuf>, Error> {
    let dirs: Vec<PathBuf> = roots
        .iter()
        .map(PathBuf::from)
        .filter(|p| p.is_absolute() && p.is_dir())
        .collect();
    if dirs.is_empty() {
        return Err(Error::Other("choose at least one existing folder".into()));
    }
    Ok(dirs)
}

// ---------------------------------------------------------------- app & system

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct AppInfo {
    version: String,
    elevated: bool,
    portable: bool,
    data_dir: String,
    os: OsInfo,
    libraries: LibraryFolders,
    suggested_roots: Vec<String>,
    settings_error: Option<String>,
    rule_problems: Vec<String>,
    user_profile: String,
}

#[tauri::command]
async fn app_info(app: AppHandle) -> AppInfo {
    let state = app.state::<AppState>();
    let (os, libraries, suggested_roots) = tauri::async_runtime::spawn_blocking(|| {
        (
            system::os_info(),
            system::library_folders(),
            artifacts::suggested_roots(),
        )
    })
    .await
    .unwrap_or_else(|_| {
        (
            OsInfo {
                name: String::new(),
                display_version: String::new(),
                build: String::new(),
            },
            system::library_folders(),
            Vec::new(),
        )
    });
    AppInfo {
        version: app.package_info().version.to_string(),
        elevated: system::is_elevated(),
        portable: is_portable(),
        data_dir: state.data_dir.to_string_lossy().into_owned(),
        os,
        libraries,
        suggested_roots,
        settings_error: state.settings_error.read().clone(),
        rule_problems: state.rule_problems.clone(),
        user_profile: std::env::var("USERPROFILE").unwrap_or_default(),
    }
}

#[tauri::command]
fn drives() -> Vec<Drive> {
    system::drives()
}

#[tauri::command]
fn system_files() -> Vec<SystemFile> {
    system::system_files()
}

#[tauri::command]
fn recycle_bin_info() -> RecycleBinInfo {
    system::recycle_bin_info()
}

#[tauri::command]
fn settings_get(state: State<'_, AppState>) -> Settings {
    state.settings.read().clone()
}

#[tauri::command]
fn settings_set(state: State<'_, AppState>, settings: Settings) -> CmdResult<Settings> {
    settings.save(&state.data_dir).map_err(err)?;
    *state.settings.write() = settings.clone();
    *state.settings_error.write() = None;
    Ok(settings)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct RulesPayload {
    targets: Vec<TargetRule>,
    artifacts: Vec<ArtifactRule>,
}

#[tauri::command]
fn rules_list(state: State<'_, AppState>) -> RulesPayload {
    RulesPayload {
        targets: state.all_rules(),
        artifacts: builtin_artifacts().to_vec(),
    }
}

#[tauri::command]
fn job_cancel(state: State<'_, AppState>, kind: String) {
    if let Some(job) = state.jobs.lock().get(&kind) {
        job.cancel();
    }
}

#[tauri::command]
fn reveal_path(path: String) -> CmdResult<()> {
    let p = Path::new(&path);
    if !p.is_absolute() || p.symlink_metadata().is_err() {
        return Err(err(Error::io(p, std::io::ErrorKind::NotFound.into())));
    }
    std::process::Command::new("explorer.exe")
        .raw_arg(format!("/select,\"{}\"", p.display()))
        .spawn()
        .map(|_| ())
        .map_err(|e| err(Error::io(p, e)))
}

#[tauri::command]
fn open_path(path: String) -> CmdResult<()> {
    let p = Path::new(&path);
    if !p.is_absolute() || !p.exists() {
        return Err(err(Error::io(p, std::io::ErrorKind::NotFound.into())));
    }
    system::shell_open(&path).map_err(err)
}

#[tauri::command]
fn restart_elevated(app: AppHandle) -> CmdResult<bool> {
    let exe = std::env::current_exe().map_err(|e| err(Error::io("current_exe", e)))?;
    match system::launch(
        &exe.to_string_lossy(),
        &format!("--wait-parent {}", std::process::id()),
        true,
    )
    .map_err(err)?
    {
        LaunchOutcome::Cancelled => Ok(false),
        LaunchOutcome::Exited(_) => {
            app.exit(0);
            Ok(true)
        }
    }
}

// ---------------------------------------------------------------- explorer

#[tauri::command]
async fn explorer_scan(app: AppHandle, root: String) -> CmdResult<ScanSummary> {
    let path = PathBuf::from(&root);
    if !path.is_absolute() || !path.is_dir() {
        return Err(err(Error::Other("folder not found".into())));
    }
    let tree = run_job(&app, "explorer", move |job| Ok(Tree::scan(&path, &job))).await?;
    let state = app.state::<AppState>();
    let summary = tree.summary.clone();
    *state.tree.write() = Some(tree);
    state.remark_tree();
    Ok(summary)
}

#[tauri::command]
fn explorer_summary(state: State<'_, AppState>) -> Option<ScanSummary> {
    state.tree.read().as_ref().map(|t| t.summary.clone())
}

#[tauri::command]
fn explorer_node(state: State<'_, AppState>, id: u32, depth: u32, limit: u32) -> Option<NodeView> {
    state
        .tree
        .read()
        .as_ref()
        .and_then(|t| t.view(id, depth.min(3), limit.clamp(4, 400) as usize))
}

#[derive(Serialize)]
struct Crumb {
    id: u32,
    name: String,
}

#[tauri::command]
fn explorer_ancestors(state: State<'_, AppState>, id: u32) -> Vec<Crumb> {
    state
        .tree
        .read()
        .as_ref()
        .map(|t| {
            t.ancestors(id)
                .into_iter()
                .map(|(id, name)| Crumb { id, name })
                .collect()
        })
        .unwrap_or_default()
}

#[tauri::command]
fn explorer_find(state: State<'_, AppState>, path: String) -> Option<u32> {
    state
        .tree
        .read()
        .as_ref()
        .and_then(|t| t.find(Path::new(&path)))
}

#[tauri::command]
fn explorer_large(state: State<'_, AppState>, min_bytes: u64, limit: u32) -> Vec<FileView> {
    state
        .tree
        .read()
        .as_ref()
        .map(|t| t.large_files(min_bytes, limit.min(5000) as usize))
        .unwrap_or_default()
}

#[tauri::command]
fn explorer_extensions(state: State<'_, AppState>, limit: u32) -> Vec<ExtStat> {
    state
        .tree
        .read()
        .as_ref()
        .map(|t| t.extensions.iter().take(limit as usize).cloned().collect())
        .unwrap_or_default()
}

// ---------------------------------------------------------------- junk

#[tauri::command]
async fn junk_scan(app: AppHandle) -> CmdResult<Vec<TargetResult>> {
    let rules = app.state::<AppState>().all_rules();
    let results = run_job(&app, "junk", move |job| {
        Ok(junk::scan_targets(&rules, &job))
    })
    .await?;
    let state = app.state::<AppState>();
    *state.junk.lock() = results.clone();
    state.remark_tree();
    Ok(results)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct CleanOutcome {
    report: CleanReport,
    elevation_cancelled: bool,
}

#[tauri::command]
async fn junk_clean(
    app: AppHandle,
    ids: Vec<String>,
    method: DeleteMethod,
) -> CmdResult<CleanOutcome> {
    let state = app.state::<AppState>();
    let guard = state.guard().map_err(err)?;
    let exclusions = state.settings.read().exclusions.clone();
    let pool = state.all_rules();
    let elevated = system::is_elevated();
    let selected: Vec<TargetRule> = clean::resolve_rules(&ids, &pool)
        .map_err(err)?
        .into_iter()
        .cloned()
        .collect();
    // Admin rules go through the elevated helper unless this process is already elevated.
    // User rules can never be admin rules, so the helper only ever sees built-in ids.
    let (admin, normal): (Vec<TargetRule>, Vec<TargetRule>) =
        selected.into_iter().partition(|r| r.admin && !elevated);
    let mut report = run_job(&app, "clean", move |job| {
        let refs: Vec<&TargetRule> = normal.iter().collect();
        Ok(clean::clean_targets(&refs, method, &guard, &job, elevated))
    })
    .await?;
    let mut elevation_cancelled = false;
    if !admin.is_empty() {
        let admin_ids: Vec<String> = admin
            .iter()
            .filter(|r| builtin_targets().iter().any(|b| b.id == r.id))
            .map(|r| r.id.clone())
            .collect();
        match tauri::async_runtime::spawn_blocking(move || {
            helper::clean_elevated(admin_ids, method, exclusions)
        })
        .await
        {
            Ok(Ok(Some(r))) => report.absorb(r),
            Ok(Ok(None)) => elevation_cancelled = true,
            Ok(Err(e)) => report.failures.push(Failure {
                path: String::from("elevated helper"),
                reason: e.to_string(),
            }),
            Err(e) => report.failures.push(Failure {
                path: String::from("elevated helper"),
                reason: e.to_string(),
            }),
        }
    }
    record(&state, "junk", method, &report);
    Ok(CleanOutcome {
        report,
        elevation_cancelled,
    })
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct CommandResult {
    code: Option<u32>,
    output: String,
    cancelled: bool,
}

/// Runs a built-in rule's official command (`npm cache clean --force`, DISM, ...). Only
/// commands from the embedded rule file can run; the UI sends a rule id, not a command line.
#[tauri::command]
async fn rule_command(app: AppHandle, id: String) -> CmdResult<CommandResult> {
    let rule = builtin_targets()
        .iter()
        .find(|r| r.id == id)
        .ok_or_else(|| err(Error::UnknownRule(id.clone())))?;
    let command = rule
        .command
        .clone()
        .filter(|c| !c.starts_with("api:"))
        .ok_or_else(|| err(Error::Other("this rule has no command".into())))?;
    let admin = rule.admin;
    let result = tauri::async_runtime::spawn_blocking(move || -> Result<CommandResult, Error> {
        if admin {
            return match system::run_and_wait("cmd.exe", &format!("/D /C {command}"), true, true)? {
                LaunchOutcome::Cancelled => Ok(CommandResult {
                    code: None,
                    output: String::new(),
                    cancelled: true,
                }),
                LaunchOutcome::Exited(code) => Ok(CommandResult {
                    code: Some(code),
                    output: String::new(),
                    cancelled: false,
                }),
            };
        }
        let out = std::process::Command::new("cmd.exe")
            .raw_arg(format!("/D /C {command}"))
            .creation_flags(CREATE_NO_WINDOW)
            .output()
            .map_err(|e| Error::io("cmd.exe", e))?;
        let mut text = String::from_utf8_lossy(&out.stdout).into_owned();
        text.push_str(&String::from_utf8_lossy(&out.stderr));
        let text: String = text
            .chars()
            .rev()
            .take(4000)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect();
        Ok(CommandResult {
            code: out.status.code().map(|c| c as u32),
            output: text,
            cancelled: false,
        })
    })
    .await
    .map_err(|e| err(Error::Other(e.to_string())))?
    .map_err(err)?;
    if !result.cancelled {
        let state = app.state::<AppState>();
        let report = CleanReport {
            items: vec![id],
            ..Default::default()
        };
        record(&state, "tool", DeleteMethod::Permanent, &report);
    }
    Ok(result)
}

// ---------------------------------------------------------------- user-picked items

#[tauri::command]
async fn projects_scan(app: AppHandle, roots: Vec<String>) -> CmdResult<Vec<Project>> {
    let dirs = existing_dirs(&roots).map_err(err)?;
    let projects = run_job(&app, "projects", move |job| {
        Ok(artifacts::scan_projects(&dirs, builtin_artifacts(), &job))
    })
    .await?;
    let state = app.state::<AppState>();
    *state.projects.lock() = projects.clone();
    state.remark_tree();
    Ok(projects)
}

#[tauri::command]
async fn duplicates_scan(
    app: AppHandle,
    roots: Vec<String>,
    min_bytes: u64,
) -> CmdResult<Vec<DupGroup>> {
    let dirs = existing_dirs(&roots).map_err(err)?;
    let guard = app.state::<AppState>().guard().map_err(err)?;
    run_job(&app, "duplicates", move |job| {
        Ok(duplicates::find_duplicates(&dirs, min_bytes, &guard, &job))
    })
    .await
}

#[tauri::command]
async fn empty_scan(app: AppHandle, roots: Vec<String>) -> CmdResult<Vec<EmptyFolder>> {
    let dirs = existing_dirs(&roots).map_err(err)?;
    let guard = app.state::<AppState>().guard().map_err(err)?;
    run_job(&app, "empty", move |job| {
        Ok(empty::find_empty(&dirs, &guard, &job))
    })
    .await
}

#[tauri::command]
async fn installers_scan(app: AppHandle) -> CmdResult<Vec<InstallerFile>> {
    let days = app.state::<AppState>().settings.read().installer_age_days;
    tauri::async_runtime::spawn_blocking(move || {
        let libs = system::library_folders();
        let dirs: Vec<PathBuf> = [libs.downloads, libs.desktop]
            .into_iter()
            .flatten()
            .map(PathBuf::from)
            .collect();
        installers::find_installers(&dirs, days)
    })
    .await
    .map_err(|e| err(Error::Other(e.to_string())))
}

#[tauri::command]
async fn paths_clean(
    app: AppHandle,
    kind: String,
    paths: Vec<String>,
    method: DeleteMethod,
) -> CmdResult<CleanReport> {
    let state = app.state::<AppState>();
    let guard = state.guard().map_err(err)?;
    // Leftovers the last uninstall reported may be removed once with the leftover guard;
    // everything else is an ordinary hand-picked delete.
    let picked: Vec<(PathBuf, Origin)> = {
        let mut leftovers = state.leftovers.lock();
        paths
            .iter()
            .map(|p| {
                let path = PathBuf::from(p);
                let origin = if kind == "apps" && leftovers.remove(&key(&path)) {
                    Origin::Leftover
                } else {
                    Origin::User
                };
                (path, origin)
            })
            .collect()
    };
    let report = run_job(&app, "clean", move |job| {
        Ok(clean::clean_picked(&picked, method, &guard, &job))
    })
    .await?;
    {
        let mut tree = state.tree.write();
        if let Some(tree) = tree.as_mut() {
            for p in &paths {
                let path = Path::new(p);
                if path.symlink_metadata().is_err() {
                    tree.remove(path);
                }
            }
        }
    }
    {
        let mut projects = state.projects.lock();
        for p in projects.iter_mut() {
            p.artifacts.retain(|a| Path::new(&a.path).exists());
            p.bytes = p.artifacts.iter().map(|a| a.bytes).sum();
        }
        projects.retain(|p| !p.artifacts.is_empty());
    }
    record(&state, &kind, method, &report);
    Ok(report)
}

#[tauri::command]
async fn recycle_bin_empty(app: AppHandle) -> CmdResult<CleanReport> {
    let state = app.state::<AppState>();
    state.guard().map_err(err)?;
    let report = tauri::async_runtime::spawn_blocking(|| {
        let before = system::recycle_bin_info();
        system::empty_recycle_bin().map(|_| CleanReport {
            freed: before.bytes,
            files: before.items,
            items: vec![String::from("recycle-bin")],
            ..Default::default()
        })
    })
    .await
    .map_err(|e| err(Error::Other(e.to_string())))?
    .map_err(err)?;
    record(&state, "recycle-bin", DeleteMethod::Permanent, &report);
    Ok(report)
}

// ---------------------------------------------------------------- apps & startup

#[tauri::command]
async fn apps_list() -> CmdResult<Vec<InstalledApp>> {
    tauri::async_runtime::spawn_blocking(apps::desktop_apps)
        .await
        .map_err(|e| err(Error::Other(e.to_string())))
}

#[tauri::command]
async fn apps_store() -> CmdResult<Vec<InstalledApp>> {
    tauri::async_runtime::spawn_blocking(apps::store_apps)
        .await
        .map_err(|e| err(Error::Other(e.to_string())))?
        .map_err(err)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct UninstallResult {
    cancelled: bool,
    exit_code: Option<u32>,
    removed: bool,
    /// Folders the app left behind, high confidence first. Empty unless it was removed.
    leftovers: Vec<Leftover>,
}

#[tauri::command]
async fn app_uninstall(app: AppHandle, id: String) -> CmdResult<UninstallResult> {
    let state = app.state::<AppState>();
    state.leftovers.lock().clear();
    // Without readable settings the user's exclusions are unknown, so no leftovers are offered.
    let guard = state.guard().ok();
    let result = tauri::async_runtime::spawn_blocking(move || -> Result<UninstallResult, Error> {
        let store = id.starts_with("store:");
        let list = if store {
            apps::store_apps()?
        } else {
            apps::desktop_apps()
        };
        let target = list
            .into_iter()
            .find(|a| a.id == id)
            .ok_or_else(|| Error::Other("the app is no longer installed".into()))?;
        let outcome = apps::uninstall(&target)?;
        if outcome == LaunchOutcome::Cancelled {
            return Ok(UninstallResult {
                cancelled: true,
                exit_code: None,
                removed: false,
                leftovers: Vec::new(),
            });
        }
        let mut remaining = if store {
            apps::store_apps()?
        } else {
            apps::desktop_apps()
        };
        let still = remaining.iter().any(|a| a.id == id);
        let leftovers = match &guard {
            Some(guard) if !still => {
                // Store apps can share a vendor folder with desktop apps, so both count.
                if store {
                    remaining.extend(apps::desktop_apps());
                }
                leftovers::find(&target, &remaining, guard)
            }
            _ => Vec::new(),
        };
        Ok(UninstallResult {
            cancelled: false,
            exit_code: match outcome {
                LaunchOutcome::Exited(c) => Some(c),
                LaunchOutcome::Cancelled => None,
            },
            removed: !still,
            leftovers,
        })
    })
    .await
    .map_err(|e| err(Error::Other(e.to_string())))?
    .map_err(err)?;
    state
        .leftovers
        .lock()
        .extend(result.leftovers.iter().map(|l| key(Path::new(&l.path))));
    Ok(result)
}

#[tauri::command]
fn startup_list() -> Vec<StartupItem> {
    startup::list()
}

#[tauri::command]
fn startup_set(source: StartupSource, name: String, enabled: bool) -> CmdResult<()> {
    startup::set_enabled(source, &name, enabled).map_err(err)
}

// ---------------------------------------------------------------- Windows tools

/// Windows' own maintenance tools. Each id maps to one fixed command line.
#[tauri::command]
async fn tool_run(app: AppHandle, id: String) -> CmdResult<CommandResult> {
    let id_for_history = id.clone();
    let res = tauri::async_runtime::spawn_blocking(move || -> Result<CommandResult, Error> {
        let done = |o: LaunchOutcome| match o {
            LaunchOutcome::Cancelled => CommandResult {
                code: None,
                output: String::new(),
                cancelled: true,
            },
            LaunchOutcome::Exited(c) => CommandResult {
                code: Some(c),
                output: String::new(),
                cancelled: false,
            },
        };
        let windir = std::env::var("WINDIR").unwrap_or_else(|_| String::from(r"C:\Windows"));
        let sys32 = format!(r"{windir}\System32");
        Ok(match id.as_str() {
            "dism-cleanup" => done(system::run_and_wait(
                &format!(r"{sys32}\Dism.exe"),
                "/Online /Cleanup-Image /StartComponentCleanup",
                true,
                true,
            )?),
            "delivery-optimization" => done(system::run_and_wait(
                &format!(r"{sys32}\WindowsPowerShell\v1.0\powershell.exe"),
                "-NoProfile -Command Delete-DeliveryOptimizationCache -Force",
                true,
                true,
            )?),
            "hibernate-off" => done(system::run_and_wait(
                &format!(r"{sys32}\powercfg.exe"),
                "/hibernate off",
                true,
                false,
            )?),
            "hibernate-on" => done(system::run_and_wait(
                &format!(r"{sys32}\powercfg.exe"),
                "/hibernate on",
                true,
                false,
            )?),
            "windows-old" => done(system::run_and_wait(
                &format!(r"{sys32}\cleanmgr.exe"),
                "/autoclean",
                true,
                true,
            )?),
            "disk-cleanup" => done(system::launch(
                &format!(r"{sys32}\cleanmgr.exe"),
                "",
                false,
            )?),
            "system-protection" => done(system::launch(
                &format!(r"{sys32}\SystemPropertiesProtection.exe"),
                "",
                false,
            )?),
            "storage-sense" => {
                system::shell_open("ms-settings:storagesense")?;
                CommandResult {
                    code: Some(0),
                    output: String::new(),
                    cancelled: false,
                }
            }
            other => return Err(Error::Other(format!("unknown tool {other}"))),
        })
    })
    .await
    .map_err(|e| err(Error::Other(e.to_string())))?
    .map_err(err)?;
    if !res.cancelled
        && matches!(
            id_for_history.as_str(),
            "dism-cleanup" | "delivery-optimization" | "hibernate-off" | "windows-old"
        )
    {
        let state = app.state::<AppState>();
        record(
            &state,
            "tool",
            DeleteMethod::Permanent,
            &CleanReport {
                items: vec![id_for_history],
                ..Default::default()
            },
        );
    }
    Ok(res)
}

// ---------------------------------------------------------------- icons & organize

#[tauri::command]
async fn shell_images(
    paths: Vec<String>,
    size: u32,
    kind: icons::ImageKind,
) -> CmdResult<HashMap<String, Option<String>>> {
    tauri::async_runtime::spawn_blocking(move || icons::images(&paths, size, kind))
        .await
        .map_err(|e| err(Error::Other(e.to_string())))
}

#[tauri::command]
async fn ai_models(app: AppHandle) -> CmdResult<Vec<organize::AiModel>> {
    let roots: Vec<PathBuf> = {
        let state = app.state::<AppState>();
        let mut r: Vec<PathBuf> = state
            .settings
            .read()
            .project_roots
            .iter()
            .map(PathBuf::from)
            .collect();
        r.extend(artifacts::suggested_roots().into_iter().map(PathBuf::from));
        r
    };
    run_job(&app, "models", move |job| {
        Ok(organize::ai_models(&roots, &job))
    })
    .await
}

#[tauri::command]
async fn ollama_remove(app: AppHandle, id: String) -> CmdResult<()> {
    tauri::async_runtime::spawn_blocking({
        let id = id.clone();
        move || organize::ollama_remove(&id)
    })
    .await
    .map_err(|e| err(Error::Other(e.to_string())))?
    .map_err(err)?;
    let state = app.state::<AppState>();
    record(
        &state,
        "models",
        DeleteMethod::Permanent,
        &CleanReport {
            items: vec![id],
            ..Default::default()
        },
    );
    Ok(())
}

#[tauri::command]
async fn assistant_data(app: AppHandle) -> CmdResult<Vec<organize::AssistantData>> {
    run_job(&app, "assistants", |job| Ok(organize::assistant_data(&job))).await
}

#[tauri::command]
async fn downloads_report(app: AppHandle) -> CmdResult<organize::DownloadsReport> {
    let dir = system::library_folders()
        .downloads
        .map(PathBuf::from)
        .filter(|p| p.is_dir())
        .ok_or_else(|| err(Error::Other("Downloads folder not found".into())))?;
    run_job(&app, "downloads", move |job| {
        Ok(organize::downloads(&dir, &job))
    })
    .await
}

#[tauri::command]
async fn app_remove_entry(app: AppHandle, id: String) -> CmdResult<String> {
    let backups = app.state::<AppState>().data_dir.join("backups");
    tauri::async_runtime::spawn_blocking(move || apps::remove_broken_entry(&id, &backups))
        .await
        .map_err(|e| err(Error::Other(e.to_string())))?
        .map_err(err)
}

// ---------------------------------------------------------------- history

#[tauri::command]
fn history_list(state: State<'_, AppState>, limit: u32) -> Vec<HistoryEntry> {
    history::read(&state.data_dir, limit as usize)
}

#[tauri::command]
fn history_clear(state: State<'_, AppState>) -> CmdResult<()> {
    history::clear(&state.data_dir).map_err(err)
}

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            if let Some(w) = app.get_webview_window("main") {
                let _ = w.unminimize();
                let _ = w.set_focus();
            }
        }))
        .plugin(tauri_plugin_window_state::Builder::default().build())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .manage(AppState::load())
        .invoke_handler(tauri::generate_handler![
            app_info,
            drives,
            system_files,
            recycle_bin_info,
            settings_get,
            settings_set,
            rules_list,
            job_cancel,
            reveal_path,
            open_path,
            restart_elevated,
            explorer_scan,
            explorer_summary,
            explorer_node,
            explorer_ancestors,
            explorer_find,
            explorer_large,
            explorer_extensions,
            junk_scan,
            junk_clean,
            rule_command,
            projects_scan,
            duplicates_scan,
            empty_scan,
            installers_scan,
            paths_clean,
            recycle_bin_empty,
            apps_list,
            apps_store,
            app_uninstall,
            startup_list,
            startup_set,
            tool_run,
            history_list,
            history_clear,
            shell_images,
            ai_models,
            ollama_remove,
            assistant_data,
            downloads_report,
            app_remove_entry,
        ])
        .run(tauri::generate_context!())
        .expect("error while running ReClaim Cleaner");
}
