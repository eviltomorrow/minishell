use criterion::{black_box, criterion_group, criterion_main, Criterion};
use minishell_store::Store;
use minishell_core::Machine;
use std::path::PathBuf;

fn test_machine(ip: &str, remark: &str) -> Machine {
    Machine {
        id: 0,
        num: 0,
        nat_ip: "".into(),
        ip: ip.into(),
        username: "root".into(),
        password: "pass".into(),
        port: 22,
        private_key_path: "".into(),
        device: "Linux".into(),
        remark: remark.into(),
    }
}

fn bench_search(c: &mut Criterion) {
    let dir = PathBuf::from(format!("/tmp/minishell_bench_{}", std::process::id()));
    let store = Store::open(&dir).unwrap();
    store.init().unwrap();
    
    // Insert 1000 machines
    let machines: Vec<Machine> = (0..1000)
        .map(|i| test_machine(
            &format!("10.0.{}.{}", i / 256, i % 256),
            &format!("server-{}", i),
        ))
        .collect();
    store.import_machines(&machines).unwrap();
    
    c.bench_function("search_all", |b| {
        b.iter(|| store.search(black_box("")))
    });
    
    c.bench_function("search_by_ip", |b| {
        b.iter(|| store.search(black_box("10.0.1")))
    });
    
    c.bench_function("search_by_remark", |b| {
        b.iter(|| store.search(black_box("server-500")))
    });
    
    std::fs::remove_dir_all(&dir).ok();
}

fn bench_import(c: &mut Criterion) {
    let dir = PathBuf::from(format!("/tmp/minishell_bench_import_{}", std::process::id()));
    let store = Store::open(&dir).unwrap();
    store.init().unwrap();
    
    let machines: Vec<Machine> = (0..100)
        .map(|i| test_machine(
            &format!("10.0.{}.{}", i / 256, i % 256),
            &format!("server-{}", i),
        ))
        .collect();
    
    c.bench_function("import_100", |b| {
        b.iter(|| {
            store.import_machines(black_box(&machines)).unwrap();
        })
    });
    
    std::fs::remove_dir_all(&dir).ok();
}

criterion_group!(benches, bench_search, bench_import);
criterion_main!(benches);
