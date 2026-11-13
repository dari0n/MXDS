//! Проверка импортированных профилей: все profiles/*.json должны
//! загружаться, проходить валидацию без ошибок и резолвиться в EnginePlan.

use mxds::strategy::{IssueSeverity, StrategyManager};
use std::path::Path;

#[test]
fn all_profiles_load_validate_resolve() {
    let mgr = StrategyManager::new(Path::new(env!("CARGO_MANIFEST_DIR")));
    let profiles = mgr.discover().expect("discover");
    assert_eq!(profiles.len(), 22, "ожидалось 22 профиля из оригинала");

    let cfg = mxds::config::Config::default();
    for p in &profiles {
        let issues = mgr.validate(p);
        let errors: Vec<_> = issues
            .iter()
            .filter(|i| i.severity == IssueSeverity::Error)
            .collect();
        assert!(errors.is_empty(), "профиль {}: {errors:?}", p.id);

        let plan = mgr.resolve(p, &cfg).unwrap_or_else(|e| {
            panic!("профиль {} не резолвится: {e}", p.id);
        });
        assert!(!plan.filter.is_empty(), "профиль {}: пустой фильтр", p.id);
        assert!(!plan.rules.is_empty(), "профиль {}: нет правил", p.id);
        // Статусы совместимости здесь не проверяем: discover() читает
        // data/profile-compat.json, а в dev-окружении там реальные
        // результаты прогонов. Дефолт NOT_TESTED и накладка статусов
        // покрыты юнит-тестами strategy::profile_compat.
    }
}

#[test]
fn profile_general_resolves_with_real_lists() {
    let mgr = StrategyManager::new(Path::new(env!("CARGO_MANIFEST_DIR")));
    let p = mgr.load("general").expect("general.json");
    let cfg = mxds::config::Config::default();
    let plan = mgr.resolve(&p, &cfg).expect("resolve");

    // general.bat: 9 правил, фильтр с Cloudflare-портами
    assert_eq!(plan.rules.len(), 9);
    assert!(plan.filter.contains("tcp.DstPort==2053"));
    assert!(plan.filter.contains("udp.DstPort>=19294"));

    // Правило с hostlist list-general: матчер существует
    let with_hostlist = plan.rules.iter().filter(|r| r.host_match.is_some()).count();
    assert!(with_hostlist >= 2, "hostlist-правил: {with_hostlist}");
}
