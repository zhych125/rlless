use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion};
use rand::{Rng, SeedableRng};
use rand_chacha::ChaCha8Rng;
use rlless::file_handler::{FileAccessor, FileAccessorFactory};
use rlless::search::{RipgrepEngine, SearchEngine, SearchOptions};
use std::fmt::Write as _;
use std::io::{BufWriter, Write};
use std::sync::Arc;
use std::time::Duration;
use tempfile::NamedTempFile;
use tokio::runtime::Runtime;

const KB: usize = 1024;
const MB: usize = 1024 * KB;

fn runtime() -> Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("failed to build tokio runtime")
}

fn write_log_fixture(mut sink: impl Write, target_bytes: usize, pattern_every: usize) -> usize {
    let mut written = 0usize;
    let mut line = String::with_capacity(160);
    let mut line_num = 0u64;

    while written < target_bytes {
        line.clear();
        let minute = (line_num / 60) % 60;
        let second = line_num % 60;

        if line_num % (pattern_every as u64) == 0 {
            let _ = writeln!(
                line,
                "2024-01-01T10:{:02}:{:02}.{:03} [Thread-{:02}] [ERROR] auth        - timeout session sess_{:08x}",
                minute,
                second,
                line_num % 1000,
                (line_num % 8) + 1,
                line_num,
            );
        } else if line_num % ((pattern_every * 10) as u64) == 0 {
            let _ = writeln!(
                line,
                "2024-01-01T10:{:02}:{:02}.{:03} [Thread-{:02}] [WARN ] payment     - Critical system alert - Memory usage at {}.{}% on server-{:03} (PID {})",
                minute,
                second,
                line_num % 1000,
                (line_num % 12) + 1,
                90 + (line_num % 10),
                (line_num % 10) * 3,
                line_num % 256,
                10_000 + line_num,
            );
        } else if line_num % ((pattern_every * 15) as u64) == 0 {
            let _ = writeln!(
                line,
                "2024-01-01T10:{:02}:{:02}.{:03} [Thread-{:02}] [INFO ] worker      - {{\"event\":\"user_action\",\"user_id\":\"usr_{:08x}\",\"action\":\"login\",\"timestamp\":{}}}",
                minute,
                second,
                line_num % 1000,
                (line_num % 16) + 1,
                line_num,
                1_609_459_200 + line_num * 60,
            );
        } else {
            let _ = writeln!(
                line,
                "2024-01-01T10:{:02}:{:02}.{:03} [Thread-{:02}] [INFO ] api         - Request {:06} processed successfully user_{:04}",
                minute,
                second,
                line_num % 1000,
                (line_num % 16) + 1,
                line_num,
                line_num % 10000,
            );
        }

        sink.write_all(line.as_bytes()).unwrap();
        written += line.len();
        line_num += 1;
    }

    written
}

fn create_fixture(size_bytes: usize, pattern_every: usize) -> NamedTempFile {
    let file = NamedTempFile::new().expect("failed to create temp log");
    let writer = std::fs::File::create(file.path()).unwrap();
    let mut buf = BufWriter::new(writer);
    write_log_fixture(&mut buf, size_bytes, pattern_every);
    buf.flush().unwrap();
    file
}

fn size_label(bytes: usize) -> String {
    if bytes >= MB {
        format!("{}MB", bytes / MB)
    } else if bytes >= KB {
        format!("{}KB", bytes / KB)
    } else {
        format!("{}B", bytes)
    }
}

fn bench_search_patterns(c: &mut Criterion) {
    let rt = runtime();
    let mut group = c.benchmark_group("search_patterns");
    group.sample_size(10);
    group.measurement_time(Duration::from_secs(5)); // Longer time for larger files

    let sizes = [256 * KB, 5 * MB, 32 * MB];
    let pattern_every = 100;

    let engines: Vec<(String, Arc<RipgrepEngine>)> = sizes
        .iter()
        .map(|&size| {
            let fixture = create_fixture(size, pattern_every);
            let accessor =
                rt.block_on(async { FileAccessorFactory::create(fixture.path()).await.unwrap() });
            let engine = RipgrepEngine::new(Arc::new(accessor) as Arc<dyn FileAccessor>);
            (size_label(size), Arc::new(engine))
        })
        .collect();

    for (label, engine) in &engines {
        let engine = Arc::clone(engine);

        // Test literal string search (most common case)
        group.bench_with_input(
            BenchmarkId::new("literal_search", label),
            &engine,
            |b, eng| {
                let options = SearchOptions::default();
                b.iter(|| {
                    let result =
                        rt.block_on(async { eng.search_from("timeout", 0, &options).await });
                    let _ = black_box(result);
                });
            },
        );

        // Test regex search (more complex)
        group.bench_with_input(
            BenchmarkId::new("regex_search", label),
            &engine,
            |b, eng| {
                let options = SearchOptions {
                    regex_mode: true,
                    ..Default::default()
                };
                b.iter(|| {
                    let result = rt.block_on(async {
                        eng.search_from(r"timeout|connection_failed", 0, &options)
                            .await
                    });
                    let _ = black_box(result);
                });
            },
        );

        // Test case-insensitive search
        group.bench_with_input(
            BenchmarkId::new("case_insensitive", label),
            &engine,
            |b, eng| {
                let options = SearchOptions {
                    case_sensitive: false,
                    ..Default::default()
                };
                b.iter(|| {
                    let result = rt.block_on(async { eng.search_from("ERROR", 0, &options).await });
                    let _ = black_box(result);
                });
            },
        );

        // Test whole word search
        group.bench_with_input(BenchmarkId::new("whole_word", label), &engine, |b, eng| {
            let options = SearchOptions {
                whole_word: true,
                ..Default::default()
            };
            b.iter(|| {
                let result = rt.block_on(async { eng.search_from("auth", 0, &options).await });
                let _ = black_box(result);
            });
        });
    }

    group.finish();
}

fn bench_search_navigation(c: &mut Criterion) {
    let rt = runtime();
    let mut group = c.benchmark_group("search_navigation");
    group.sample_size(10);
    group.measurement_time(Duration::from_secs(2));

    let size_kb = 20000; // 20MB file for navigation testing
    let pattern_frequency = 50; // More frequent patterns for navigation

    let fixture = create_fixture(size_kb * KB, pattern_frequency);
    let accessor =
        rt.block_on(async { FileAccessorFactory::create(fixture.path()).await.unwrap() });
    let engine = Arc::new(RipgrepEngine::new(
        Arc::new(accessor) as Arc<dyn FileAccessor>
    ));

    // Test forward navigation (n command in less)
    group.bench_function("search_next", |b| {
        let options = SearchOptions::default();
        let engine = Arc::clone(&engine);
        b.iter(|| {
            // Start from middle of file
            let result = rt.block_on(async { engine.search_from("timeout", 1000, &options).await });
            let _ = black_box(result);
        });
    });

    // Test backward navigation (N command in less)
    group.bench_function("search_prev", |b| {
        let options = SearchOptions::default();
        let engine = Arc::clone(&engine);
        b.iter(|| {
            // Start from near end of file
            let result = rt.block_on(async { engine.search_prev("timeout", 2000, &options).await });
            let _ = black_box(result);
        });
    });

    // Test search with context (like grep -C)
    group.bench_function("search_with_context", |b| {
        let options = SearchOptions {
            ..Default::default()
        };
        let engine = Arc::clone(&engine);
        b.iter(|| {
            let result =
                rt.block_on(async { engine.search_from("connection_failed", 0, &options).await });
            let _ = black_box(result);
        });
    });

    group.finish();
}

fn bench_search_caching(c: &mut Criterion) {
    let rt = runtime();
    let mut group = c.benchmark_group("search_caching");
    group.sample_size(10);
    group.measurement_time(Duration::from_secs(2));

    let size_kb = 10000; // 10MB file
    let pattern_frequency = 75;

    let fixture = create_fixture(size_kb * KB, pattern_frequency);
    let accessor =
        rt.block_on(async { FileAccessorFactory::create(fixture.path()).await.unwrap() });
    let engine = Arc::new(RipgrepEngine::new(
        Arc::new(accessor) as Arc<dyn FileAccessor>
    ));

    // Warm up cache with first search
    let options = SearchOptions::default();
    rt.block_on(async {
        let _ = engine.search_from("timeout", 0, &options).await;
    });

    // Test cache hit performance
    group.bench_function("cached_search", |b| {
        let engine = Arc::clone(&engine);
        b.iter(|| {
            let result = rt.block_on(async { engine.search_from("timeout", 0, &options).await });
            let _ = black_box(result);
        });
    });

    // Test cache miss performance (new pattern)
    group.bench_function("uncached_search", |b| {
        let mut counter = 0;
        let engine = Arc::clone(&engine);
        b.iter(|| {
            counter += 1;
            let pattern = format!("user_{}", counter % 1000);
            let result = rt.block_on(async { engine.search_from(&pattern, 0, &options).await });
            let _ = black_box(result);
        });
    });

    group.finish();
}

fn bench_complex_regex_patterns(c: &mut Criterion) {
    let rt = runtime();
    let mut group = c.benchmark_group("complex_regex");
    group.sample_size(10);
    group.measurement_time(Duration::from_secs(8)); // Longer time for complex patterns

    // Test with larger files that have sparse matches
    let sizes = [5 * MB, 20 * MB, 64 * MB];
    let pattern_frequency = 100; // Every 100 lines has basic patterns

    for &size in &sizes {
        let fixture = create_fixture(size, pattern_frequency);
        let accessor =
            rt.block_on(async { FileAccessorFactory::create(fixture.path()).await.unwrap() });
        let engine = Arc::new(RipgrepEngine::new(
            Arc::new(accessor) as Arc<dyn FileAccessor>
        ));

        let size_label = size_label(size);

        // Test 1: Complex IPv4 address pattern (matches ~1 in 1000 lines)
        group.bench_with_input(
            BenchmarkId::new("ipv4_pattern", &size_label),
            &engine,
            |b, eng| {
                let options = SearchOptions {
                    regex_mode: true,
                    ..Default::default()
                };
                b.iter(|| {
                    let result = rt.block_on(async {
                        eng.search_from(r"IPv4: 192\.168\.1\.\d{1,3}", 0, &options)
                            .await
                    });
                    let _ = black_box(result);
                });
            },
        );

        // Test 2: Complex memory usage pattern (matches ~1 in 1000 lines)
        group.bench_with_input(
            BenchmarkId::new("memory_alert", &size_label),
            &engine,
            |b, eng| {
                let options = SearchOptions {
                    regex_mode: true,
                    ..Default::default()
                };
                b.iter(|| {
                    let result = rt.block_on(async {
                        eng.search_from(
                            r"Memory usage at \d{2}\.\d%.*server-\d{3}.*PID: \d+",
                            0,
                            &options,
                        )
                        .await
                    });
                    let _ = black_box(result);
                });
            },
        );

        // Test 3: JSON structure pattern (matches ~1 in 1500 lines)
        group.bench_with_input(
            BenchmarkId::new("json_structure", &size_label),
            &engine,
            |b, eng| {
                let options = SearchOptions {
                    regex_mode: true,
                    ..Default::default()
                };
                b.iter(|| {
                    let result = rt.block_on(async {
                        eng.search_from(
                            r#"\{"event":"user_action".*"user_id":"usr_[0-9a-f]{8}".*"timestamp":\d{10}"#,
                            0,
                            &options,
                        )
                        .await
                    });
                    let _ = black_box(result);
                });
            },
        );

        // Test 4: Multi-line pattern simulation (complex lookahead-like pattern)
        group.bench_with_input(
            BenchmarkId::new("session_pattern", &size_label),
            &engine,
            |b, eng| {
                let options = SearchOptions {
                    regex_mode: true,
                    ..Default::default()
                };
                b.iter(|| {
                    let result = rt.block_on(async {
                        eng.search_from(r"Session: sess_[0-9a-f]+.*Critical.*Memory", 0, &options)
                            .await
                    });
                    let _ = black_box(result);
                });
            },
        );

        // Test 5: Complex timestamp and service correlation
        group.bench_with_input(
            BenchmarkId::new("correlation_pattern", &size_label),
            &engine,
            |b, eng| {
                let options = SearchOptions {
                    regex_mode: true,
                    ..Default::default()
                };
                b.iter(|| {
                    let result = rt.block_on(async {
                        eng.search_from(
                            r"\[2024-09-02T10:[0-5]\d:[0-5]\d\].*auth.*usr_[0-9a-f]{8}.*timestamp",
                            0,
                            &options,
                        )
                        .await
                    });
                    let _ = black_box(result);
                });
            },
        );
    }

    group.finish();
}

fn bench_random_start_positions(c: &mut Criterion) {
    let rt = runtime();
    let mut group = c.benchmark_group("random_start_search");
    group.sample_size(10);
    group.measurement_time(Duration::from_secs(6)); // Longer time for random access

    // Test with large files to see real random access patterns
    let sizes = [5 * MB, 20 * MB, 64 * MB];
    let pattern_frequency = 100;

    for &size in &sizes {
        let fixture = create_fixture(size, pattern_frequency);
        let accessor =
            rt.block_on(async { FileAccessorFactory::create(fixture.path()).await.unwrap() });

        let size_label = size_label(size);

        let file_size = accessor.file_size();

        let engine = Arc::new(RipgrepEngine::new(
            Arc::new(accessor) as Arc<dyn FileAccessor>
        ));

        // Test 1: Random start literal search
        group.bench_with_input(
            BenchmarkId::new("literal_random", &size_label),
            &engine,
            |b, eng| {
                let options = SearchOptions::default();
                let mut rng = ChaCha8Rng::seed_from_u64(42); // Fixed seed for reproducibility
                b.iter(|| {
                    // Generate random start byte position (avoid last 10% to ensure matches)
                    let start_byte = rng.gen_range(0..file_size.saturating_sub(file_size / 10));
                    let result = rt
                        .block_on(async { eng.search_from("timeout", start_byte, &options).await });
                    let _ = black_box(result);
                });
            },
        );

        // Test 2: Random start regex search
        group.bench_with_input(
            BenchmarkId::new("regex_random", &size_label),
            &engine,
            |b, eng| {
                let options = SearchOptions {
                    regex_mode: true,
                    ..Default::default()
                };
                let mut rng = ChaCha8Rng::seed_from_u64(43); // Different seed
                b.iter(|| {
                    let start_byte = rng.gen_range(0..file_size.saturating_sub(file_size / 10));
                    let result = rt.block_on(async {
                        eng.search_from(
                            r"\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}",
                            start_byte,
                            &options,
                        )
                        .await
                    });
                    let _ = black_box(result);
                });
            },
        );

        // Test 3: Random start complex IPv4 pattern (sparse matches)
        group.bench_with_input(
            BenchmarkId::new("ipv4_random", &size_label),
            &engine,
            |b, eng| {
                let options = SearchOptions {
                    regex_mode: true,
                    ..Default::default()
                };
                let mut rng = ChaCha8Rng::seed_from_u64(44); // Different seed
                b.iter(|| {
                    let start_byte = rng.gen_range(0..file_size.saturating_sub(file_size / 5));
                    let result = rt.block_on(async {
                        eng.search_from(r"IPv4: 192\.168\.1\.\d{1,3}", start_byte, &options)
                            .await
                    });
                    let _ = black_box(result);
                });
            },
        );

        // Test 4: Random start backward search
        group.bench_with_input(
            BenchmarkId::new("backward_random", &size_label),
            &engine,
            |b, eng| {
                let options = SearchOptions::default();
                let mut rng = ChaCha8Rng::seed_from_u64(45); // Different seed
                b.iter(|| {
                    // For backward search, start from middle to end of file
                    let start_byte = rng.gen_range(file_size / 2..file_size);
                    let result = rt
                        .block_on(async { eng.search_prev("timeout", start_byte, &options).await });
                    let _ = black_box(result);
                });
            },
        );

        // Test 5: Random middle search with context (realistic user behavior)
        group.bench_with_input(
            BenchmarkId::new("context_random", &size_label),
            &engine,
            |b, eng| {
                let options = SearchOptions {
                    ..Default::default()
                };
                let mut rng = ChaCha8Rng::seed_from_u64(46); // Different seed
                b.iter(|| {
                    let start_byte = rng.gen_range(file_size / 4..3 * file_size / 4);
                    let result =
                        rt.block_on(async { eng.search_from("ERROR", start_byte, &options).await });
                    let _ = black_box(result);
                });
            },
        );
    }

    group.finish();
}

criterion_group!(
    benches,
    bench_search_patterns,
    bench_search_navigation,
    bench_search_caching,
    bench_complex_regex_patterns,
    bench_random_start_positions
);
criterion_main!(benches);
