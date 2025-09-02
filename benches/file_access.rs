use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion};
use flate2::{write::GzEncoder, Compression};
use rlless::file_handler::FileAccessorFactory;
use std::io::Write;
use tempfile::NamedTempFile;
use tokio::runtime::Runtime;

fn create_test_file(size_kb: usize) -> NamedTempFile {
    let mut temp_file = NamedTempFile::new().expect("Failed to create temp file");
    let target_size = size_kb * 1024;
    let mut current_size = 0;
    let mut line_num = 0;
    
    while current_size < target_size {
        let log_line = format!("[2024-09-02T10:{}:{}] INFO: Request {} user_{}\n",
                              (line_num / 3600) % 24, (line_num / 60) % 60, line_num, line_num % 1000);
        temp_file.write_all(log_line.as_bytes()).unwrap();
        current_size += log_line.len();
        line_num += 1;
    }
    
    temp_file.flush().unwrap();
    temp_file
}

fn create_compressed_test_file(size_kb: usize) -> NamedTempFile {
    let target_size = size_kb * 1024;
    let mut content = Vec::new();
    let mut current_size = 0;
    let mut line_num = 0;
    
    while current_size < target_size {
        let log_line = format!("[2024-09-02T10:{}:{}] INFO: Request {} user_{}\n",
                              (line_num / 3600) % 24, (line_num / 60) % 60, line_num, line_num % 1000);
        content.extend_from_slice(log_line.as_bytes());
        current_size += log_line.len();
        line_num += 1;
    }
    
    let compressed_file = NamedTempFile::new().unwrap();
    let file = std::fs::File::create(compressed_file.path()).unwrap();
    let mut encoder = GzEncoder::new(file, Compression::default());
    encoder.write_all(&content).unwrap();
    encoder.finish().unwrap();
    compressed_file
}

fn bench_file_opening(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();
    let mut group = c.benchmark_group("file_opening");
    group.sample_size(10);
    group.measurement_time(std::time::Duration::from_secs(5));
    
    // Include larger files to test mmap accessor (>50MB threshold on macOS, >10MB on other platforms)
    let sizes_kb = [50, 500, 5000, 20000, 60000]; // 50KB, 500KB, 5MB, 20MB, 60MB
    
    for &size_kb in &sizes_kb {
        let temp_file = create_test_file(size_kb);
        let size_label = if size_kb < 1024 { 
            format!("{}KB", size_kb) 
        } else { 
            format!("{}MB", size_kb / 1024) 
        };
        
        group.bench_with_input(
            BenchmarkId::new("uncompressed", &size_label),
            &temp_file.path(),
            |b, path| {
                b.iter(|| {
                    let accessor = rt.block_on(async {
                        FileAccessorFactory::create(path).await.unwrap()
                    });
                    black_box(accessor.file_size());
                });
            },
        );
        
        let compressed_file = create_compressed_test_file(size_kb);
        group.bench_with_input(
            BenchmarkId::new("compressed", &size_label),
            &compressed_file.path(),
            |b, path| {
                b.iter(|| {
                    let accessor = rt.block_on(async {
                        FileAccessorFactory::create(path).await.unwrap()
                    });
                    black_box(accessor.file_size());
                });
            },
        );
    }
    
    group.finish();
}

fn bench_line_access(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();
    let mut group = c.benchmark_group("line_access");
    group.sample_size(10);
    group.measurement_time(std::time::Duration::from_secs(3));
    
    // Test both InMemory and Mmap accessors
    let sizes_kb = [500, 20000, 60000]; // 500KB (InMemory), 20MB (InMemory on macOS), 60MB (Mmap)
    
    for &size_kb in &sizes_kb {
        let temp_file = create_test_file(size_kb);
        let accessor = rt.block_on(async {
            FileAccessorFactory::create(temp_file.path()).await.unwrap()
        });
        
        let size_label = if size_kb < 1024 { 
            format!("{}KB", size_kb) 
        } else { 
            format!("{}MB", size_kb / 1024) 
        };
        
        group.bench_with_input(
            BenchmarkId::new("uncompressed", &size_label),
            &accessor,
            |b, acc| {
                b.iter(|| {
                    let line = rt.block_on(async {
                        acc.read_line(0).await.unwrap()
                    });
                    black_box(line);
                });
            },
        );
        
        let compressed_file = create_compressed_test_file(size_kb);
        let compressed_accessor = rt.block_on(async {
            FileAccessorFactory::create(compressed_file.path()).await.unwrap()
        });
        
        let compressed_size_label = if size_kb < 1024 { 
            format!("{}KB", size_kb) 
        } else { 
            format!("{}MB", size_kb / 1024) 
        };
        
        group.bench_with_input(
            BenchmarkId::new("compressed", &compressed_size_label),
            &compressed_accessor,
            |b, acc| {
                b.iter(|| {
                    let line = rt.block_on(async {
                        acc.read_line(0).await.unwrap()
                    });
                    black_box(line);
                });
            },
        );
    }
    
    group.finish();
}

criterion_group!(benches, bench_file_opening, bench_line_access);
criterion_main!(benches);
