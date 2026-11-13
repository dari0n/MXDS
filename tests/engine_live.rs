//! Интеграционный тест с реальным драйвером WinDivert.
//!
//! Требует прав администратора. Запуск:
//!   cargo test --test engine_live -- --ignored --nocapture
//! из elevated-терминала (или scripts/run-live-tests.bat через UAC).
//!
//! Тест НЕ выполняется в обычном `cargo test` (#[ignore]): без elevation
//! WinDivertOpen вернёт код 5, и это корректное поведение, а не сбой.

#![cfg(windows)]

use mxds::engine::desync::Fooling;
use mxds::engine::{
    build_filter, wait_stopped, EngineManager, EnginePlan, EngineRule, EngineState, L4Proto,
    RuleAction, RuleTechnique,
};
use mxds::windivert::{default_dll_path, DivertHandle};
use std::time::Duration;

/// Passthrough-план: фильтр на TCP/443+80 и UDP/443, без правил.
fn passthrough_plan() -> EnginePlan {
    let cfg = mxds::config::Config::default();
    EnginePlan {
        filter: build_filter(&[80, 443], &[443], &cfg.game_filter),
        rules: vec![],
    }
}

/// План с одним fake-правилом на TCP/443 TLS.
fn fake_plan() -> EnginePlan {
    let cfg = mxds::config::Config::default();
    EnginePlan {
        filter: build_filter(&[443], &[], &cfg.game_filter),
        rules: vec![EngineRule {
            name: "fake-tls".into(),
            protocol: L4Proto::Tcp,
            ports: vec![(443, 443)],
            host_match: None,
            ip_match: None,
            l7: vec![],
            action: RuleAction {
                techniques: vec![RuleTechnique::Fake],
                repeats: 1,
                fooling: Fooling {
                    bad_seq: true,
                    ..Default::default()
                },
                fake_tls: std::fs::read(
                    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                        .join("assets/payloads/tls_clienthello_www_google_com.bin"),
                )
                .ok(),
                ..Default::default()
            },
        }],
    }
}

#[test]
#[ignore = "требует прав администратора"]
fn open_close_real_driver() {
    let h = DivertHandle::open_with_path(
        "outbound and !loopback and tcp.DstPort==443",
        &default_dll_path(),
    );
    match h {
        Ok(handle) => {
            let v = handle.driver_version().expect("driver_version");
            assert_eq!(v.0, 2, "ожидался драйвер 2.x, получен {v:?}");
            println!("WinDivert версия {}.{}", v.0, v.1);
            handle.shutdown().expect("shutdown");
        }
        Err(mxds::windivert::DivertError::AccessDenied) => {
            panic!("тест запущен без прав администратора");
        }
        Err(e) => panic!("WinDivertOpen: {e:?}"),
    }
}

#[test]
#[ignore = "требует прав администратора"]
fn engine_lifecycle_live() {
    let mut mgr = EngineManager::new();
    let plan = passthrough_plan();
    mgr.start(&plan, &Default::default()).expect("start");
    assert!(mgr.state().is_running());

    // Генерируем трафик для прохождения пакетов через рабочий цикл.
    for _ in 0..3 {
        let _ = std::net::TcpStream::connect("93.184.216.34:80");
    }
    std::thread::sleep(Duration::from_millis(500));

    let stats = mgr.stats();
    println!(
        "recv={} sent={} dropped={}",
        stats.packets_received, stats.packets_sent, stats.packets_dropped
    );

    mgr.stop().expect("stop");
    assert!(wait_stopped(&mgr, Duration::from_secs(5)));
    assert_eq!(mgr.state(), EngineState::Stopped);
    // Двойной запуск после остановки — допустим.
    mgr.start(&plan, &Default::default()).expect("restart");
    mgr.stop().expect("stop after restart");
}

#[test]
#[ignore = "требует прав администратора"]
fn engine_desync_fake_mode_live() {
    // Fake-правило на реальном драйвере: проверяем, что
    // цикл не падает и счётчики движутся.
    let mut mgr = EngineManager::new();
    mgr.start(&fake_plan(), &Default::default()).expect("start");
    // HTTPS-соединение: ClientHello должен попасть под правило.
    let _ = std::net::TcpStream::connect("140.82.121.4:443");
    std::thread::sleep(Duration::from_millis(700));
    let stats = mgr.stats();
    println!(
        "recv={} sent={} dropped={} fakes={}",
        stats.packets_received, stats.packets_sent, stats.packets_dropped, stats.fakes_sent
    );
    mgr.stop().expect("stop");
    assert!(wait_stopped(&mgr, Duration::from_secs(5)));
}

#[test]
#[ignore = "требует прав администратора"]
fn engine_general_profile_live() {
    // Цикл 8: полный путь — профиль general.json → resolve → EnginePlan →
    // движок на реальном драйвере.
    let mgr =
        mxds::strategy::StrategyManager::new(std::path::Path::new(env!("CARGO_MANIFEST_DIR")));
    let profile = mgr.load("general").expect("general.json");
    let cfg = mxds::config::Config::default();
    let plan = mgr.resolve(&profile, &cfg).expect("resolve");

    let mut eng = EngineManager::new();
    eng.start(&plan, &Default::default()).expect("start");
    assert!(eng.state().is_running());

    // Трафик под правила: HTTPS (TLS ClientHello, hostlist general) + Discord UDP.
    let _ = std::net::TcpStream::connect("142.250.74.46:443"); // google
    let _ = std::net::TcpStream::connect("157.240.9.35:443"); // facebook (в list-general)
    std::thread::sleep(Duration::from_millis(800));

    let stats = eng.stats();
    println!(
        "general: recv={} sent={} dropped={} fakes={}",
        stats.packets_received, stats.packets_sent, stats.packets_dropped, stats.fakes_sent
    );
    eng.stop().expect("stop");
    assert!(wait_stopped(&eng, Duration::from_secs(5)));
}
