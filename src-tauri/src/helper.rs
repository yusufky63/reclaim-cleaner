//! Elevated helper.
//!
//! When the user cleans rules that need administrator rights from a normal session, the app
//! writes a small request file and starts its own exe with `--elevated-clean <file>` through
//! UAC. The request carries rule ids and options only, never paths: the helper expands the
//! built-in rules itself, so a tampered request cannot point it at anything else.

use std::path::{Path, PathBuf};

use reclaim_core::clean::{CleanReport, DeleteMethod, clean_targets, resolve_rules};
use reclaim_core::paths::Guard;
use reclaim_core::rules::builtin_targets;
use reclaim_core::settings::data_dir;
use reclaim_core::system::{LaunchOutcome, is_elevated, run_and_wait};
use reclaim_core::{Error, Job, Result};
use serde::{Deserialize, Serialize};
use windows::Win32::Foundation::CloseHandle;
use windows::Win32::System::Threading::{OpenProcess, PROCESS_SYNCHRONIZE, WaitForSingleObject};

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HelperRequest {
    pub ids: Vec<String>,
    pub method: DeleteMethod,
    pub exclusions: Vec<String>,
}

fn result_path(request: &Path) -> PathBuf {
    request.with_extension("result.json")
}

/// Entry point of `--elevated-clean`. Returns the process exit code.
pub fn run(request: Option<&str>) -> i32 {
    let Some(request) = request.map(PathBuf::from) else {
        return 2;
    };
    let parsed: std::result::Result<HelperRequest, String> = std::fs::read_to_string(&request)
        .map_err(|e| e.to_string())
        .and_then(|s| serde_json::from_str(&s).map_err(|e| e.to_string()));
    let Ok(req) = parsed else { return 3 };
    let report = match resolve_rules(&req.ids, builtin_targets()) {
        Ok(rules) => {
            let guard = Guard::new(&req.exclusions, Some(&data_dir()));
            clean_targets(&rules, req.method, &guard, &Job::new(), is_elevated())
        }
        Err(_) => return 4,
    };
    // The result goes next to the request, into a folder that must be a real folder, and the
    // file must not exist yet, so an elevated write cannot be redirected.
    let out = result_path(&request);
    let parent_ok = out
        .parent()
        .and_then(|p| p.symlink_metadata().ok())
        .is_some_and(|m| m.is_dir() && !m.file_type().is_symlink());
    if !parent_ok {
        return 5;
    }
    let Ok(file) = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&out)
    else {
        return 6;
    };
    if serde_json::to_writer(file, &report).is_err() {
        return 7;
    }
    0
}

/// Runs the helper through UAC and returns its report. `Ok(None)` means the user declined.
pub fn clean_elevated(
    ids: Vec<String>,
    method: DeleteMethod,
    exclusions: Vec<String>,
) -> Result<Option<CleanReport>> {
    let dir = data_dir().join("elevated");
    std::fs::create_dir_all(&dir).map_err(|e| Error::io(&dir, e))?;
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let request = dir.join(format!("request-{stamp}.json"));
    std::fs::write(
        &request,
        serde_json::to_vec(&HelperRequest {
            ids,
            method,
            exclusions,
        })?,
    )
    .map_err(|e| Error::io(&request, e))?;
    let exe = std::env::current_exe().map_err(|e| Error::io("current_exe", e))?;
    let outcome = run_and_wait(
        &exe.to_string_lossy(),
        &format!("--elevated-clean \"{}\"", request.display()),
        true,
        false,
    );
    let result = result_path(&request);
    let report = match outcome {
        Ok(LaunchOutcome::Cancelled) => Ok(None),
        Ok(LaunchOutcome::Exited(0)) => std::fs::read_to_string(&result)
            .map_err(|e| Error::io(&result, e))
            .and_then(|s| serde_json::from_str::<CleanReport>(&s).map_err(Error::from))
            .map(Some),
        Ok(LaunchOutcome::Exited(code)) => Err(Error::Other(format!(
            "the elevated helper failed with code {code}"
        ))),
        Err(e) => Err(e),
    };
    let _ = std::fs::remove_file(&request);
    let _ = std::fs::remove_file(&result);
    report
}

pub fn wait_for_process(pid: u32) {
    unsafe {
        if let Ok(h) = OpenProcess(PROCESS_SYNCHRONIZE, false, pid) {
            let _ = WaitForSingleObject(h, 10_000);
            let _ = CloseHandle(h);
        }
    }
}
