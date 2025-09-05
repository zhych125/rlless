use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion};
use rand::{Rng, SeedableRng};
use rand_chacha::ChaCha8Rng;
use rlless::file_handler::FileAccessorFactory;
use rlless::search::{RipgrepEngine, SearchEngine, SearchOptions};
use std::io::Write;
use std::time::Duration;
use tempfile::NamedTempFile;
use tokio::runtime::Runtime;

fn create_log_file_with_patterns(size_kb: usize, pattern_frequency: usize) -> NamedTempFile {
    let mut temp_file = NamedTempFile::new().expect("Failed to create temp file");
    let target_size = size_kb * 1024;
    let mut current_size = 0;
    let mut line_num = 0;

    // Create realistic log patterns
    let log_levels = ["DEBUG", "INFO", "WARN", "ERROR", "FATAL"];
    let services = ["auth", "database", "cache", "payment", "notification"];
    let error_patterns = [
        "timeout",
        "connection_failed",
        "null_pointer",
        "out_of_memory",
    ];

    while current_size < target_size {
        let log_level = log_levels[line_num % log_levels.len()];
        let service = services[line_num % services.len()];

        let log_line = if line_num % pattern_frequency == 0 {
            // Insert search pattern every N lines
            let error = error_patterns[line_num / pattern_frequency % error_patterns.len()];
            format!(
                "[2024-09-02T10:{}:{:02}] {} {}: Request {} failed with {} user_{}\n",
                (line_num / 3600) % 24,
                (line_num / 60) % 60,
                log_level,
                service,
                line_num,
                error,
                line_num % 1000
            )
        } else if line_num % (pattern_frequency * 10) == 0 {
            // Add rare complex patterns every 1000 lines
            format!(
                "[2024-09-02T10:{}:{:02}] {} {}: Critical system alert - Memory usage at 95.7% on server-{:03} (PID: {}) - IPv4: 192.168.1.{} - Session: sess_{:x}\n",
                (line_num / 3600) % 24,
                (line_num / 60) % 60,
                log_level,
                service,
                line_num % 256,
                line_num + 1000,
                line_num % 254 + 1,
                line_num
            )
        } else if line_num % (pattern_frequency * 15) == 0 {
            // Add JSON-like structured logs every 1500 lines
            format!(
                "[2024-09-02T10:{}:{:02}] {} {}: {{\"event\":\"user_action\",\"user_id\":\"{}\",\"action\":\"login\",\"timestamp\":{},\"metadata\":{{\"browser\":\"Chrome/118.0\",\"os\":\"Linux\"}}}}\n",
                (line_num / 3600) % 24,
                (line_num / 60) % 60,
                log_level,
                service,
                format!("usr_{:08x}", line_num),
                1609459200 + line_num * 60
            )
        } else {
            format!(
                "[2024-09-02T10:{}:{:02}] {} {}: Request {} processed successfully user_{}\n",
                (line_num / 3600) % 24,
                (line_num / 60) % 60,
                log_level,
                service,
                line_num,
                line_num % 1000
            )
        };

        temp_file.write_all(log_line.as_bytes()).unwrap();
        current_size += log_line.len();
        line_num += 1;
    }

    temp_file.flush().unwrap();
    temp_file
}

fn bench_search_patterns(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();
    let mut group = c.benchmark_group("search_patterns");
    group.sample_size(10);
    group.measurement_time(Duration::from_secs(5)); // Longer time for larger files

    // Test different file sizes to validate <500ms target for large files
    // Include sizes that trigger MmapFileAccessor (>50MB on macOS, >10MB on other platforms)
    let sizes_kb = [100, 500, 5000, 15000, 60000]; // 100KB, 500KB, 5MB, 15MB, 60MB
    let pattern_frequency = 100; // Pattern every 100 lines

    for &size_kb in &sizes_kb {
        let temp_file = create_log_file_with_patterns(size_kb, pattern_frequency);
        let accessor =
            rt.block_on(async { FileAccessorFactory::create(temp_file.path()).await.unwrap() });
        let engine = RipgrepEngine::new(accessor.into());

        let size_label = if size_kb < 1024 {
            format!("{}KB", size_kb)
        } else {
            format!("{}MB", size_kb / 1024)
        };

        // Test literal string search (most common case)
        group.bench_with_input(
            BenchmarkId::new("literal_search", &size_label),
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
            BenchmarkId::new("regex_search", &size_label),
            &engine,
            |b, eng| {
                let options = SearchOptions {
                    regex_mode: true,
                    ..Default::default()
                };
                b.iter(|| {
                    let result = rt.block_on(async {
                        eng.search_from(r"\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}", 0, &options)
                            .await
                    });
                    let _ = black_box(result);
                });
            },
        );

        // Test case-insensitive search
        group.bench_with_input(
            BenchmarkId::new("case_insensitive", &size_label),
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
        group.bench_with_input(
            BenchmarkId::new("whole_word", &size_label),
            &engine,
            |b, eng| {
                let options = SearchOptions {
                    whole_word: true,
                    ..Default::default()
                };
                b.iter(|| {
                    let result = rt.block_on(async { eng.search_from("auth", 0, &options).await });
                    let _ = black_box(result);
                });
            },
        );
    }

    group.finish();
}

fn bench_search_navigation(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();
    let mut group = c.benchmark_group("search_navigation");
    group.sample_size(10);
    group.measurement_time(Duration::from_secs(2));

    let size_kb = 20000; // 20MB file for navigation testing
    let pattern_frequency = 50; // More frequent patterns for navigation

    let temp_file = create_log_file_with_patterns(size_kb, pattern_frequency);
    let accessor =
        rt.block_on(async { FileAccessorFactory::create(temp_file.path()).await.unwrap() });
    let engine = RipgrepEngine::new(accessor.into());

    // Test forward navigation (n command in less)
    group.bench_function("search_next", |b| {
        let options = SearchOptions::default();
        b.iter(|| {
            // Start from middle of file
            let result = rt.block_on(async { engine.search_from("timeout", 1000, &options).await });
            let _ = black_box(result);
        });
    });

    // Test backward navigation (N command in less)
    group.bench_function("search_prev", |b| {
        let options = SearchOptions::default();
        b.iter(|| {
            // Start from near end of file
            let result = rt.block_on(async { engine.search_prev("timeout", 2000, &options).await });
            let _ = black_box(result);
        });
    });

    // Test search with context (like grep -C)
    group.bench_function("search_with_context", |b| {
        let options = SearchOptions {
            context_lines: 3,
            ..Default::default()
        };
        b.iter(|| {
            let result =
                rt.block_on(async { engine.search_from("connection_failed", 0, &options).await });
            let _ = black_box(result);
        });
    });

    group.finish();
}

fn bench_search_caching(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();
    let mut group = c.benchmark_group("search_caching");
    group.sample_size(10);
    group.measurement_time(Duration::from_secs(2));

    let size_kb = 10000; // 10MB file
    let pattern_frequency = 75;

    let temp_file = create_log_file_with_patterns(size_kb, pattern_frequency);
    let accessor =
        rt.block_on(async { FileAccessorFactory::create(temp_file.path()).await.unwrap() });
    let engine = RipgrepEngine::new(accessor.into());

    // Warm up cache with first search
    let options = SearchOptions::default();
    rt.block_on(async {
        let _ = engine.search_from("timeout", 0, &options).await;
    });

    // Test cache hit performance
    group.bench_function("cached_search", |b| {
        b.iter(|| {
            let result = rt.block_on(async { engine.search_from("timeout", 0, &options).await });
            let _ = black_box(result);
        });
    });

    // Test cache miss performance (new pattern)
    group.bench_function("uncached_search", |b| {
        let mut counter = 0;
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
    let rt = Runtime::new().unwrap();
    let mut group = c.benchmark_group("complex_regex");
    group.sample_size(10);
    group.measurement_time(Duration::from_secs(8)); // Longer time for complex patterns

    // Test with larger files that have sparse matches
    let sizes_kb = [5000, 15000, 60000]; // 5MB, 15MB, 60MB
    let pattern_frequency = 100; // Every 100 lines has basic patterns

    for &size_kb in &sizes_kb {
        let temp_file = create_log_file_with_patterns(size_kb, pattern_frequency);
        let accessor =
            rt.block_on(async { FileAccessorFactory::create(temp_file.path()).await.unwrap() });
        let engine = RipgrepEngine::new(accessor.into());

        let size_label = if size_kb < 1024 {
            format!("{}KB", size_kb)
        } else {
            format!("{}MB", size_kb / 1024)
        };

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
    let rt = Runtime::new().unwrap();
    let mut group = c.benchmark_group("random_start_search");
    group.sample_size(10);
    group.measurement_time(Duration::from_secs(6)); // Longer time for random access

    // Test with large files to see real random access patterns
    let sizes_kb = [5000, 15000, 60000]; // 5MB, 15MB, 60MB
    let pattern_frequency = 100;

    for &size_kb in &sizes_kb {
        let temp_file = create_log_file_with_patterns(size_kb, pattern_frequency);
        let accessor =
            rt.block_on(async { FileAccessorFactory::create(temp_file.path()).await.unwrap() });
        
        let size_label = if size_kb < 1024 {
            format!("{}KB", size_kb)
        } else {
            format!("{}MB", size_kb / 1024)
        };

        // Estimate line count based on file size (average ~60 chars per line)
        let file_size = accessor.file_size();
        let estimated_lines = (file_size / 60).max(100); // Minimum 100 lines for safety
        
        let engine = RipgrepEngine::new(accessor.into());

        // Test 1: Random start literal search
        group.bench_with_input(
            BenchmarkId::new("literal_random", &size_label),
            &engine,
            |b, eng| {
                let options = SearchOptions::default();
                let mut rng = ChaCha8Rng::seed_from_u64(42); // Fixed seed for reproducibility
                b.iter(|| {
                    // Generate random start position (avoid last 10% to ensure matches)
                    let start_line = rng.gen_range(0..estimated_lines.saturating_sub(estimated_lines / 10));
                    let result = rt.block_on(async {
                        eng.search_from("timeout", start_line, &options).await
                    });
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
                    let start_line = rng.gen_range(0..estimated_lines.saturating_sub(estimated_lines / 10));
                    let result = rt.block_on(async {
                        eng.search_from(r"\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}", start_line, &options)
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
                    let start_line = rng.gen_range(0..estimated_lines.saturating_sub(estimated_lines / 5));
                    let result = rt.block_on(async {
                        eng.search_from(r"IPv4: 192\.168\.1\.\d{1,3}", start_line, &options)
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
                    let start_line = rng.gen_range(estimated_lines / 2..estimated_lines);
                    let result = rt.block_on(async {
                        eng.search_prev("timeout", start_line, &options).await
                    });
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
                    context_lines: 2,
                    ..Default::default()
                };
                let mut rng = ChaCha8Rng::seed_from_u64(46); // Different seed
                b.iter(|| {
                    let start_line = rng.gen_range(estimated_lines / 4..3 * estimated_lines / 4);
                    let result = rt.block_on(async {
                        eng.search_from("ERROR", start_line, &options).await
                    });
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
