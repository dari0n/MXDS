//! End-to-End тесты CLI (Цикл 15): прогон собранного бинарника mxds
//! с изолированным корнем (MXDS_ROOT → temp). Сеть и elevation
//! не требуются; опасных сетевых экспериментов нет.

use std::path::{Path, PathBuf};
use std::process::Command;

/// Изолированный корень приложения: копии config/, profiles/, lists/.
/// TempDir «забывается» — живёт до конца процесса теста.
fn isolated_root() -> PathBuf {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().to_path_buf();

    fn copy_dir(from: &Path, to: &Path) {
        for entry in std::fs::read_dir(from).unwrap() {
            let entry = entry.unwrap();
            let dest = to.join(entry.file_name());
            if entry.file_type().unwrap().is_dir() {
                std::fs::create_dir_all(&dest).unwrap();
                copy_dir(&entry.path(), &dest);
            } else {
                std::fs::create_dir_all(to).unwrap();
                std::fs::copy(entry.path(), &dest).unwrap();
            }
        }
    }
    copy_dir(&manifest.join("config"), &root.join("config"));
    copy_dir(&manifest.join("profiles"), &root.join("profiles"));
    copy_dir(&manifest.join("lists"), &root.join("lists"));
    // Пользовательский config.json из репозитория не копируем —
    // тест должен работать от дефолтов.
    let _ = std::fs::remove_file(root.join("config").join("config.json"));
    std::mem::forget(tmp);
    root
}

fn mxds() -> Command {
    Command::new(env!("CARGO_BIN_EXE_mxds"))
}

fn mxds_at(root: &Path, args: &[&str]) -> std::process::Output {
    mxds()
        .args(args)
        .env("MXDS_ROOT", root)
        .output()
        .expect("запуск mxds")
}

#[test]
fn version_prints_name_and_version() {
    let out = mxds().arg("version").output().expect("запуск mxds");
    assert_eq!(out.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("mxds"), "stdout: {stdout}");
    assert!(
        stdout.contains(env!("CARGO_PKG_VERSION")),
        "stdout: {stdout}"
    );
}

#[test]
fn unknown_subcommand_is_exit_2() {
    let out = mxds().arg("frobnicate").output().expect("запуск mxds");
    assert_eq!(out.status.code(), Some(2));
}

#[test]
fn status_without_running_instance() {
    let root = isolated_root();
    let out = mxds_at(&root, &["status"]);
    assert_eq!(
        out.status.code(),
        Some(0),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("не запущен"), "stdout: {stdout}");
}

#[test]
fn profiles_list_includes_general() {
    let root = isolated_root();
    let out = mxds_at(&root, &["profiles", "list"]);
    assert_eq!(
        out.status.code(),
        Some(0),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("general"), "stdout: {stdout}");
}

#[test]
fn config_show_outputs_valid_json() {
    let root = isolated_root();
    let out = mxds_at(&root, &["config", "show"]);
    assert_eq!(
        out.status.code(),
        Some(0),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let json: serde_json::Value =
        serde_json::from_slice(&out.stdout).expect("config show должен выводить JSON");
    assert!(json.is_object());
    assert!(json.get("log_level").is_some(), "json: {json}");
}

#[test]
fn config_validate_ok_on_defaults() {
    let root = isolated_root();
    let out = mxds_at(&root, &["config", "validate"]);
    assert_eq!(
        out.status.code(),
        Some(0),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn config_with_too_new_schema_fails_bootstrap() {
    // default.json со schema_version новее поддерживаемой — bootstrap
    // обязан завершиться кодом 1 с сообщением (а не паникой).
    let tmp = tempfile::tempdir().unwrap();
    let cfg_dir = tmp.path().join("config");
    std::fs::create_dir_all(&cfg_dir).unwrap();
    std::fs::write(cfg_dir.join("default.json"), r#"{ "schema_version": 999 }"#).unwrap();
    let out = mxds_at(tmp.path(), &["config", "show"]);
    assert_eq!(out.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("mxds:"), "stderr: {stderr}");
}

#[test]
fn run_without_admin_rights_is_exit_4() {
    // В непривилегированном тестовом процессе run обязан завершиться
    // кодом 4 (нет прав). Если тест запущен elevated — пропускаем.
    if mxds::windows::is_elevated() {
        eprintln!("тест пропущен: процесс elevated");
        return;
    }
    let root = isolated_root();
    let out = mxds_at(&root, &["run"]);
    assert_eq!(
        out.status.code(),
        Some(4),
        "stdout: {} stderr: {}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
}
