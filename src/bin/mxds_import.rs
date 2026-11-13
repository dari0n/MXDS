//! Утилита разработки (Цикл 8): одноразовый импорт стратегий оригинала
//! (.bat с winws-аргументами) в типизированные JSON-профили MXDS.
//!
//! Запуск:
//!   cargo run --bin mxds_import -- <каталог .bat оригинала> <каталог profiles/>
//!
//! Импорт — миграционный шаг, не рантайм-функция (MODULE_CONTRACTS п.6).

use mxds::strategy::{extract_winws_args, parse_winws_args, slug};

fn main() {
    let mut args = std::env::args().skip(1);
    let Some(src) = args.next() else {
        eprintln!("usage: mxds_import <dir-with-bat> <profiles-dir>");
        std::process::exit(2);
    };
    let Some(dst) = args.next() else {
        eprintln!("usage: mxds_import <dir-with-bat> <profiles-dir>");
        std::process::exit(2);
    };

    std::fs::create_dir_all(&dst).expect("create profiles dir");
    let mut imported = 0;
    let mut failed = 0;

    let mut entries: Vec<_> = std::fs::read_dir(&src)
        .expect("read src dir")
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("bat"))
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .map(|n| n.starts_with("general"))
                .unwrap_or(false)
        })
        .collect();
    entries.sort();

    for path in entries {
        let name = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("unknown")
            .to_string();
        let content = match std::fs::read_to_string(&path) {
            Ok(c) => c,
            Err(e) => {
                // bat в CP866/UTF-8 — пробуем lossy
                eprintln!("WARN {name}: read_to_string: {e}, пробую lossy");
                String::from_utf8_lossy(&std::fs::read(&path).expect("read bytes")).into_owned()
            }
        };
        let Some(args) = extract_winws_args(&content) else {
            eprintln!("FAIL {name}: вызов winws.exe не найден");
            failed += 1;
            continue;
        };
        match parse_winws_args(&name, &args) {
            Ok((profile, unsupported)) => {
                let id = slug(&name);
                let out = std::path::Path::new(&dst).join(format!("{id}.json"));
                let json = serde_json::to_string_pretty(&profile).expect("serialize");
                std::fs::write(&out, json).expect("write profile");
                println!(
                    "OK   {name} -> {id}.json (rules={}, unsupported={:?})",
                    profile.rules.len(),
                    unsupported
                );
                imported += 1;
            }
            Err(e) => {
                eprintln!("FAIL {name}: {e}");
                failed += 1;
            }
        }
    }
    println!("Импортировано: {imported}, ошибок: {failed}");
    if failed > 0 {
        std::process::exit(1);
    }
}
