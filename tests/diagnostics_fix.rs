//! E2E-автотест цикла «диагностика → исправление → повторная диагностика»:
//! после успешного run_fix проверка обязана перейти в Pass — иначе в GUI
//! кнопка «Исправить» визуально «не исправляет» проблему.
//!
//! Изменяющие систему проверки требуют прав администратора — без них
//! тесты самопропускаются (аналогично live-тестам движка).

use mxds::diagnostics::{default_checks, run_all, run_fix, CheckStatus, DiagContext, FixId};

fn standalone_ctx() -> DiagContext {
    DiagContext::standalone(std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")))
}

#[test]
fn fixes_only_attached_to_failed_checks() {
    let ctx = standalone_ctx();
    for r in run_all(&ctx, default_checks()) {
        if r.status == CheckStatus::Pass {
            assert!(r.fix.is_none(), "у успешной проверки {} есть fix", r.name);
        }
    }
}

#[test]
fn tcp_timestamps_fix_flips_check_to_pass() {
    if !mxds::windows::is_elevated() {
        eprintln!("пропуск: нужны права администратора");
        return;
    }
    let before = run_all(&standalone_ctx(), default_checks());
    let ts = before
        .iter()
        .find(|r| r.name == "tcp_timestamps")
        .expect("проверка tcp_timestamps есть в наборе");
    if ts.status == CheckStatus::Pass {
        return; // уже включены — проверять цикл нечего
    }

    assert_eq!(ts.fix, Some(FixId::EnableTcpTimestamps));
    run_fix(FixId::EnableTcpTimestamps).expect("исправление TCP timestamps");

    let after = run_all(&standalone_ctx(), default_checks());
    let ts = after.iter().find(|r| r.name == "tcp_timestamps").unwrap();
    assert_eq!(
        ts.status,
        CheckStatus::Pass,
        "после исправления проверка должна стать Pass, а не {:?}",
        ts.status
    );
    assert!(ts.fix.is_none(), "после исправления fix не предлагается");
}

#[test]
fn secure_dns_fix_flips_check_to_pass() {
    if !mxds::windows::is_elevated() {
        eprintln!("пропуск: нужны права администратора");
        return;
    }
    let before = run_all(&standalone_ctx(), default_checks());
    let dns = before
        .iter()
        .find(|r| r.name == "secure_dns")
        .expect("проверка secure_dns есть в наборе");
    if dns.status == CheckStatus::Pass {
        return; // DoH уже настроен
    }

    assert_eq!(dns.fix, Some(FixId::EnableSecureDns));
    // Исправление честное: либо включает DoH и проверка зеленеет,
    // либо возвращает понятную ошибку (нет активных адаптеров и т.п.).
    match run_fix(FixId::EnableSecureDns) {
        Ok(msg) => {
            let after = run_all(&standalone_ctx(), default_checks());
            let dns = after.iter().find(|r| r.name == "secure_dns").unwrap();
            assert_eq!(
                dns.status,
                CheckStatus::Pass,
                "fix сообщил успех ({msg}), но проверка {:?}",
                dns.status
            );
        }
        Err(e) => {
            // Ошибка исправления — легитимный исход, но текст обязан
            // объяснять причину и следующий шаг.
            let text = e.to_string();
            assert!(
                text.contains("1.1.1.1") || text.contains("адапт"),
                "непонятная ошибка: {text}"
            );
        }
    }
}
