use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion};
use flate2::{write::GzEncoder, Compression};
use rlless::file_handler::{FileAccessor, FileAccessorFactory};
use std::fmt::Write as _;
use std::io::{BufWriter, Write};
use std::sync::Arc;
use tempfile::NamedTempFile;
use tokio::runtime::Runtime;

const KB: usize = 1024;
const MB: usize = 1024 * KB;

#[derive(Clone, Copy)]
enum FixtureKind {
    Plain,
    Gzip,
}

fn write_fixture(mut sink: impl Write, target_bytes: usize) -> usize {
    let mut written = 0usize;
    let mut line_num = 0u64;

    while written < target_bytes {
        let timestamp_min = (line_num / 60) % 60;
        let timestamp_sec = line_num % 60;
        let mut line = String::with_capacity(120);
        let _ = writeln!(
            line,
            "2024-01-01T10:{:02}:{:02}.{:03} [Thread-{:02}] [INFO ] api        - request {:06} user_{:04}",
            timestamp_min,
            timestamp_sec,
            line_num % 1000,
            (line_num % 16) + 1,
            line_num,
            line_num % 10000
        );
        sink.write_all(line.as_bytes()).unwrap();
        written += line.len();
        line_num += 1;
    }

    written
}

fn create_fixture(size_bytes: usize, kind: FixtureKind) -> NamedTempFile {
    let temp = NamedTempFile::new().expect("failed to create temp file");
    match kind {
        FixtureKind::Plain => {
            let file = std::fs::File::create(temp.path()).unwrap();
            let mut writer = BufWriter::new(file);
            write_fixture(&mut writer, size_bytes);
            writer.flush().unwrap();
        }
        FixtureKind::Gzip => {
            let file = std::fs::File::create(temp.path()).unwrap();
            let mut encoder = GzEncoder::new(file, Compression::new(5));
            write_fixture(&mut encoder, size_bytes);
            encoder.finish().unwrap();
        }
    }
    temp
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

fn runtime() -> Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("failed to build tokio runtime")
}

fn bench_file_opening(c: &mut Criterion) {
    let rt = runtime();
    let mut group = c.benchmark_group("file_opening");
    group.sample_size(10);
    group.measurement_time(std::time::Duration::from_secs(4));

    let sizes = [512 * KB, 8 * MB, 64 * MB];

    let fixtures_plain: Vec<(String, NamedTempFile)> = sizes
        .iter()
        .map(|&size| (size_label(size), create_fixture(size, FixtureKind::Plain)))
        .collect();

    let fixtures_gzip: Vec<(String, NamedTempFile)> = sizes
        .iter()
        .map(|&size| (size_label(size), create_fixture(size, FixtureKind::Gzip)))
        .collect();

    for (label, fixture) in &fixtures_plain {
        let path = fixture.path().to_path_buf();
        group.bench_with_input(BenchmarkId::new("plain", label), &path, |b, p| {
            b.iter(|| {
                let accessor = rt.block_on(async { FileAccessorFactory::create(p).await.unwrap() });
                black_box(accessor.file_size());
            });
        });
    }

    for (label, fixture) in &fixtures_gzip {
        let path = fixture.path().to_path_buf();
        group.bench_with_input(BenchmarkId::new("gzip", label), &path, |b, p| {
            b.iter(|| {
                let accessor = rt.block_on(async { FileAccessorFactory::create(p).await.unwrap() });
                black_box(accessor.file_size());
            });
        });
    }

    group.finish();
}

fn bench_line_access(c: &mut Criterion) {
    let rt = runtime();
    let mut group = c.benchmark_group("line_access");
    group.sample_size(10);
    group.measurement_time(std::time::Duration::from_secs(3));

    let sizes = [2 * MB, 64 * MB];

    let plain_accessors: Vec<(String, Arc<dyn FileAccessor>)> = sizes
        .iter()
        .map(|&size| {
            let fixture = create_fixture(size, FixtureKind::Plain);
            let accessor =
                rt.block_on(async { FileAccessorFactory::create(fixture.path()).await.unwrap() });
            (
                size_label(size),
                Arc::new(accessor) as Arc<dyn FileAccessor>,
            )
        })
        .collect();

    for (label, accessor) in &plain_accessors {
        group.bench_with_input(BenchmarkId::new("plain", label), accessor, |b, acc| {
            b.iter(|| {
                let lines = rt.block_on(async { acc.read_from_byte(0, 64).await.unwrap() });
                black_box(lines);
            });
        });
    }

    let gzip_accessors: Vec<(String, Arc<dyn FileAccessor>)> = sizes
        .iter()
        .map(|&size| {
            let fixture = create_fixture(size, FixtureKind::Gzip);
            let accessor =
                rt.block_on(async { FileAccessorFactory::create(fixture.path()).await.unwrap() });
            (
                size_label(size),
                Arc::new(accessor) as Arc<dyn FileAccessor>,
            )
        })
        .collect();

    for (label, accessor) in &gzip_accessors {
        group.bench_with_input(BenchmarkId::new("gzip", label), accessor, |b, acc| {
            b.iter(|| {
                let lines = rt.block_on(async { acc.read_from_byte(0, 64).await.unwrap() });
                black_box(lines);
            });
        });
    }

    group.finish();
}

criterion_group!(benches, bench_file_opening, bench_line_access);
criterion_main!(benches);
