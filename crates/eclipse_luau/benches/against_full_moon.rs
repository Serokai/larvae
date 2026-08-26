/*!
The same corpus through full_moon, for the comparison the README cites.

Run with: cargo bench -p eclipse_luau --features full-moon-bench
*/

#[cfg(feature = "full-moon-bench")]
mod bench {
    use criterion::{Criterion, criterion_group, criterion_main};

    fn corpus() -> Vec<String> {
        let dir = std::env::var("BENCH_CORPUS").unwrap_or_else(|_| {
            concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/tests/fixtures/luau-conformance"
            )
            .to_string()
        });

        std::fs::read_dir(dir)
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|e| std::fs::read_to_string(e.path()).ok())
            .collect()
    }

    fn benches(c: &mut Criterion) {
        let corpus = corpus();

        // full_moon refuses what it cannot parse; both sides skip those
        // files, so the two measure the same inputs.
        let sources: Vec<&str> = corpus
            .iter()
            .map(String::as_str)
            .filter(|s| full_moon::parse(s).is_ok())
            .collect();

        let mut group = c.benchmark_group("against_full_moon");

        group.bench_function("eclipse_luau", |b| {
            b.iter(|| {
                for src in &sources {
                    let _ = std::hint::black_box(eclipse_luau::parse_one(src));
                }
            })
        });

        group.bench_function("full_moon", |b| {
            b.iter(|| {
                for src in &sources {
                    let _ = std::hint::black_box(full_moon::parse(src));
                }
            })
        });

        group.finish();
    }

    criterion_group!(cmp, benches);
    criterion_main!(cmp);
}

#[cfg(feature = "full-moon-bench")]
fn main() {
    bench::cmp();
}

#[cfg(not(feature = "full-moon-bench"))]
fn main() {}
