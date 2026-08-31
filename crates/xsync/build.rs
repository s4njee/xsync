use std::env;
use std::fs;
use std::path::Path;

fn main() {
    println!("cargo:rerun-if-changed=resources/xsync-server.cmd");
    stamp_provenance();

    let out_dir = env::var_os("OUT_DIR").expect("Cargo must provide OUT_DIR");
    let profile_dir = Path::new(&out_dir)
        .parent()
        .and_then(Path::parent)
        .and_then(Path::parent)
        .expect("OUT_DIR must be below target/<profile>/build/<package>");
    let destination = profile_dir.join("xsync-server.cmd");

    let launcher = fs::read_to_string("resources/xsync-server.cmd")
        .unwrap_or_else(|error| panic!("cannot read resources/xsync-server.cmd: {error}"));
    assert!(
        launcher.contains("%~dp0xs.exe"),
        "Windows server launcher must invoke the xs.exe binary"
    );

    fs::copy("resources/xsync-server.cmd", &destination)
        .unwrap_or_else(|error| panic!("cannot package {}: {error}", destination.display()));
}

/// Record what this binary was built from, so a bug report can name it exactly.
///
/// The version itself is deliberately *not* stamped here: it comes from
/// `CARGO_PKG_VERSION`, which Cargo reads from the manifest. A second source
/// could disagree with the manifest, and a binary whose reported version
/// differs from the tag it shipped under is worse than no version at all.
fn stamp_provenance() {
    watch_git_state();
    println!("cargo:rerun-if-env-changed=SOURCE_DATE_EPOCH");

    println!("cargo:rustc-env=XSYNC_BUILD_COMMIT={}", git_commit());
    println!("cargo:rustc-env=XSYNC_BUILD_DATE={}", build_date());
    println!(
        "cargo:rustc-env=XSYNC_BUILD_TARGET={}",
        env::var("TARGET").unwrap_or_else(|_| "unknown".to_owned())
    );
}

/// Short commit hash, suffixed `-dirty` when the tree has uncommitted changes.
///
/// `unknown` when git is unavailable or this is not a checkout — a source
/// tarball build is legitimate and must not fail for want of a commit.
fn git_commit() -> String {
    let Some(hash) = run_git(&["rev-parse", "--short=12", "HEAD"]) else {
        return "unknown".to_owned();
    };
    // `--porcelain` is empty exactly when the working tree is clean. A dirty
    // marker matters: "it reproduces on abc123" is misleading if abc123 was
    // built with uncommitted edits.
    match run_git(&["status", "--porcelain", "--untracked-files=no"]) {
        Some(status) if !status.is_empty() => format!("{hash}-dirty"),
        _ => hash,
    }
}

/// Re-run this script whenever the recorded commit could change.
///
/// Watching `.git/HEAD` alone is not enough, and that was the bug: on a branch
/// that file holds `ref: refs/heads/main` and does not change when commits land
/// on it. The stamp went stale for every commit after the first, so `--version`
/// named a commit that was not the one compiled -- the one tool for diagnosing
/// version skew being itself a source of it.
///
/// So follow `HEAD` to the ref it names and watch that too, plus `packed-refs`
/// (where the ref file lives once git has packed it, in which case the loose
/// file does not exist) and `index` (which moves on `git add`, keeping the
/// `-dirty` marker roughly honest).
///
/// The commit is exact. The `-dirty` marker is best-effort: an uncommitted edit
/// that never reaches the index does not re-trigger this script, so a binary
/// can report clean while containing modified source. It is a hint that a build
/// was not reproducible, not a guarantee that one was.
fn watch_git_state() {
    let Some(git_dir) = git_dir() else {
        return;
    };
    let head = git_dir.join("HEAD");
    watch(&head);
    watch(&git_dir.join("packed-refs"));
    watch(&git_dir.join("index"));

    // `ref: refs/heads/<branch>` on a branch; a bare hash when detached, which
    // needs no further watching because the hash is already in the file.
    if let Ok(contents) = fs::read_to_string(&head) {
        if let Some(reference) = contents.trim().strip_prefix("ref:") {
            watch(&git_dir.join(reference.trim()));
        }
    }
}

/// Locate the git directory, or `None` outside a checkout.
///
/// `.git` is a *file* in a worktree or submodule, holding `gitdir: <path>`.
/// Resolving it matters because a build from a worktree would otherwise watch
/// nothing and stamp a commit that never updates.
fn git_dir() -> Option<std::path::PathBuf> {
    let candidates = [Path::new(".git"), Path::new("../../.git")];
    for candidate in candidates {
        if candidate.is_dir() {
            return Some(candidate.to_path_buf());
        }
        if candidate.is_file() {
            let contents = fs::read_to_string(candidate).ok()?;
            let target = contents.trim().strip_prefix("gitdir:")?.trim();
            let resolved = Path::new(target).to_path_buf();
            let resolved = if resolved.is_absolute() {
                resolved
            } else {
                candidate.parent()?.join(resolved)
            };
            if resolved.is_dir() {
                return Some(resolved);
            }
        }
    }
    None
}

/// Declare a rerun trigger for a path that may not exist yet.
///
/// Cargo accepts a missing path and re-runs if it later appears, which is what
/// makes watching both `packed-refs` and a loose ref correct: exactly one of
/// them exists at any time, and packing switches between them.
fn watch(path: &Path) {
    println!("cargo:rerun-if-changed={}", path.display());
}

fn run_git(args: &[&str]) -> Option<String> {
    let output = std::process::Command::new("git").args(args).output().ok()?;
    if !output.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

/// Build date as `YYYY-MM-DD` UTC.
///
/// Honours `SOURCE_DATE_EPOCH`, the cross-ecosystem convention for reproducible
/// builds: without it the timestamp alone would make two builds of the same
/// commit differ, which is precisely what story D2.2 has to rule out.
fn build_date() -> String {
    let seconds = env::var("SOURCE_DATE_EPOCH")
        .ok()
        .and_then(|value| value.trim().parse::<i64>().ok())
        .unwrap_or_else(|| {
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|elapsed| i64::try_from(elapsed.as_secs()).unwrap_or_default())
                .unwrap_or_default()
        });
    format_utc_date(seconds)
}

/// Civil date from a Unix timestamp, without pulling in a date library for one
/// string. Days-from-epoch to y/m/d via Howard Hinnant's algorithm.
///
/// `doe`/`doy` (day-of-era, day-of-year) are the published names for these
/// quantities. Renaming them to satisfy `similar_names` would make the code
/// harder to check against the algorithm it came from, not easier.
#[allow(clippy::similar_names)]
fn format_utc_date(seconds: i64) -> String {
    let days = seconds.div_euclid(86_400);
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!("{y:04}-{m:02}-{d:02}")
}
