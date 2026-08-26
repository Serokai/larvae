/*!
Parse throughput over a corpus.

The default corpus is the vendored Luau conformance suite, so the bench
runs on a fresh clone with no setup. Point BENCH_CORPUS at a directory of
.luau files to measure a real codebase; the numbers that matter come from
one of those.

The full_moon comparison lives in `against_full_moon.rs`, behind the
`full-moon-bench` feature, so nothing here builds it by accident.
*/

use criterion::{Criterion, criterion_group, criterion_main};

fn corpus() -> Vec<(String, String)> {
    let dir = std::env::var("BENCH_CORPUS").unwrap_or_else(|_| {
        concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../larvae/tests/fixtures/parser/luau-conformance"
        )
        .to_string()
    });

    let mut files = Vec::new();
    let mut queue = vec![std::path::PathBuf::from(dir)];

    while let Some(dir) = queue.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };

        for entry in entries.flatten() {
            let path = entry.path();

            if path.is_dir() {
                queue.push(path);
            } else if path.extension().is_some_and(|e| e == "luau" || e == "lua")
                && let Ok(text) = std::fs::read_to_string(&path)
            {
                files.push((path.display().to_string(), text));
            }
        }
    }

    files
}

fn benches(c: &mut Criterion) {
    let corpus = corpus();
    let bytes: usize = corpus.iter().map(|(_, t)| t.len()).sum();
    let sources: Vec<&str> = corpus.iter().map(|(_, t)| t.as_str()).collect();

    let mut group = c.benchmark_group("parse");
    group.throughput(criterion::Throughput::Bytes(bytes as u64));

    group.bench_function("sequential", |b| {
        b.iter(|| {
            for src in &sources {
                let _ = std::hint::black_box(eclipse_luau::parse_one(src));
            }
        })
    });

    group.bench_function("parse_many", |b| {
        b.iter(|| std::hint::black_box(eclipse_luau::parse_many(&sources)))
    });

    group.finish();
}

criterion_group!(parse, benches);
criterion_main!(parse);
