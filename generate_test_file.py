#!/usr/bin/env python3
"""Utility to generate large log files (plain text or gzip compressed).

The script produces realistic log-like content with timestamps, log levels,
and search-friendly patterns.  It now supports command-line arguments so you
can control the output size and whether the file is compressed.  This makes it
handy for generating multi-gigabyte fixtures (e.g. a 20 GB `.gz` archive) for
performance testing.
"""

from __future__ import annotations

import argparse
import datetime
import gzip
import os
import random
from typing import Optional

def generate_log_line(line_num):
    """Generate a realistic log line."""

    # Log levels with weights (more INFO, fewer ERROR)
    levels = ['INFO'] * 50 + ['DEBUG'] * 30 + ['WARN'] * 15 + ['ERROR'] * 5
    level = random.choice(levels)

    # Generate timestamp
    base_time = datetime.datetime(2024, 1, 1, 0, 0, 0)
    time_offset = datetime.timedelta(seconds=line_num * 0.1)
    timestamp = (base_time + time_offset).strftime('%Y-%m-%d %H:%M:%S.%f')[:-3]

    # Generate different types of messages
    message_templates = [
        "Processing request from user_{} with session_id={}",
        "Database query executed in {}ms for table '{}'",
        "Cache {} for key '{}' (size: {} bytes)",
        "API endpoint {} called with {} parameters",
        "Background job '{}' completed in {}s",
        "Memory usage: {}MB / {}MB ({}% utilized)",
        "Connection established to {} on port {}",
        "File '{}' uploaded successfully ({} KB)",
        "Authentication {} for user '{}' from IP {}",
        "Queue depth: {} messages pending processing",
        "Transaction {} completed with status: {}",
        "Service health check: {} services online",
        "Metrics collected: {} data points in {}ms",
        "Configuration reloaded from '{}'",
        "SSL certificate {} for domain '{}'",
        "Rate limit {} for endpoint '{}': {}/min",
        "Backup {} completed for database '{}'",
        "Index rebuilt for collection '{}' ({} documents)",
        "WebSocket connection {} from client '{}'",
        "Email sent to {} recipients via SMTP server '{}'",
    ]

    # Special patterns to make search interesting
    if line_num % 100 == 0:
        message = f"MILESTONE: Processed {line_num} records"
    elif line_num % 500 == 0:
        message = f"CHECKPOINT: System status nominal at line {line_num}"
    elif level == 'ERROR':
        error_messages = [
            f"Connection timeout after {random.randint(1000, 5000)}ms",
            f"Failed to parse JSON at position {random.randint(1, 1000)}",
            f"Database deadlock detected in transaction {random.randint(10000, 99999)}",
            f"Out of memory: tried to allocate {random.randint(100, 1000)}MB",
            f"Permission denied for resource '/data/{random.choice(['users', 'logs', 'cache'])}'",
            f"Invalid token: signature verification failed",
            f"Service unavailable: circuit breaker open",
            f"CRITICAL: Disk usage at {random.randint(85, 99)}%"
        ]
        message = random.choice(error_messages)
    elif level == 'WARN':
        warn_messages = [
            f"Slow query detected: {random.randint(1000, 5000)}ms",
            f"Cache miss rate high: {random.randint(20, 40)}%",
            f"Connection pool near capacity: {random.randint(80, 95)}% used",
            f"Deprecated API version {random.choice(['v1', 'v2'])} still in use",
            f"Memory pressure detected, GC triggered",
            f"Rate limiting applied to IP {random.randint(1, 255)}.{random.randint(1, 255)}.{random.randint(1, 255)}.{random.randint(1, 255)}"
        ]
        message = random.choice(warn_messages)
    else:
        template = random.choice(message_templates)

        # Fill in template with random values
        if '{}' in template:
            fill_values = []
            for _ in range(template.count('{}')):
                fill_type = random.choice(['number', 'string', 'status', 'ip'])
                if fill_type == 'number':
                    fill_values.append(str(random.randint(1, 10000)))
                elif fill_type == 'string':
                    fill_values.append(random.choice(['user_data', 'session_info', 'products', 'orders', 'analytics', 'metrics', 'logs']))
                elif fill_type == 'status':
                    fill_values.append(random.choice(['success', 'completed', 'pending', 'processing', 'cached']))
                else:  # ip
                    fill_values.append(f"{random.randint(1, 255)}.{random.randint(1, 255)}.{random.randint(1, 255)}.{random.randint(1, 255)}")
            message = template.format(*fill_values)

    # Add some keywords that are good for searching
    if random.random() < 0.01:  # 1% chance
        message += " [IMPORTANT]"
    if random.random() < 0.005:  # 0.5% chance
        message += " [SECURITY]"
    if random.random() < 0.002:  # 0.2% chance
        message += " [PERFORMANCE]"

    # Add thread/process ID
    thread_id = f"[Thread-{random.randint(1, 16):02d}]"

    # Add module name
    modules = ['api', 'database', 'cache', 'auth', 'worker', 'scheduler', 'monitor', 'network']
    module = random.choice(modules)

    # Construct the full log line
    return f"{timestamp} {thread_id} [{level:5s}] {module:10s} - {message}"

def parse_size_bytes(size_str: str) -> int:
    """Parse a human-friendly size string (e.g. '20G', '512M')."""

    size_str = size_str.strip().lower()
    if not size_str:
        raise ValueError("Size string cannot be empty")

    multipliers = {
        'k': 1024,
        'm': 1024 ** 2,
        'g': 1024 ** 3,
        't': 1024 ** 4,
    }

    suffix = size_str[-1]
    if suffix.isdigit():
        return int(float(size_str))

    if suffix not in multipliers:
        raise ValueError(f"Unrecognised size suffix '{suffix}' in '{size_str}'")

    value = float(size_str[:-1])
    if value <= 0:
        raise ValueError("Size must be positive")

    return int(value * multipliers[suffix])


def write_logs(
    output_path: str,
    target_bytes: int,
    compressed: bool,
    compresslevel: int,
) -> tuple[int, int, int]:
    """Generate logs until the uncompressed byte budget is reached.

    Returns a tuple of (lines_written, uncompressed_bytes, compressed_bytes).
    """

    line_count = 0
    uncompressed_bytes = 0

    if compressed:
        f = gzip.open(output_path, 'wt', compresslevel=compresslevel)
    else:
        f = open(output_path, 'w')

    try:
        while uncompressed_bytes < target_bytes:
            line = generate_log_line(line_count) + '\n'
            f.write(line)
            encoded_length = len(line.encode('utf-8'))
            uncompressed_bytes += encoded_length
            line_count += 1

            if line_count % 100000 == 0:
                progress = (uncompressed_bytes / target_bytes) * 100
                print(
                    f"  Progress: {progress:6.2f}% "
                    f"({uncompressed_bytes / 1024 / 1024:.1f} MiB uncompressed)"
                    f" - {line_count:,} lines",
                    end='\r',
                )
    finally:
        f.close()

    compressed_bytes = os.path.getsize(output_path)
    return line_count, uncompressed_bytes, compressed_bytes


def build_arg_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--output",
        default="large_test_file.log",
        help="Path to the output file (defaults to %(default)s)",
    )
    parser.add_argument(
        "--size",
        default="1G",
        help=(
            "Uncompressed data to write (e.g. '20G', '512M'). "
            "This controls how much log content is generated; gzipped files may end up smaller."
        ),
    )
    parser.add_argument(
        "--gzip",
        action="store_true",
        help="Write gzip-compressed output (suffix '.gz' is not added automatically)",
    )
    parser.add_argument(
        "--compress-level",
        type=int,
        default=6,
        help="Gzip compression level (1=fast, 9=best). Ignored for plain output.",
    )
    return parser


def main() -> None:
    parser = build_arg_parser()
    args = parser.parse_args()

    try:
        target_bytes = parse_size_bytes(args.size)
    except ValueError as exc:
        parser.error(str(exc))

    mode = "gzip" if args.gzip else "plain"
    print(f"Generating {args.size.upper()} of log data -> {args.output} ({mode})")

    lines, uncompressed, compressed = write_logs(
        output_path=args.output,
        target_bytes=target_bytes,
        compressed=args.gzip,
        compresslevel=args.compress_level,
    )

    print("\nGeneration complete:")
    print(f"  Lines written        : {lines:,}")
    print(f"  Uncompressed payload : {uncompressed / 1024 / 1024:.2f} MiB")
    if args.gzip:
        print(f"  Compressed size      : {compressed / 1024 / 1024:.2f} MiB")

    print("\nUseful search patterns:")
    print("  /ERROR       - Find error messages")
    print("  /CRITICAL    - Find critical errors")
    print("  /MILESTONE   - Find milestone markers (every 100 lines)")
    print("  /CHECKPOINT  - Find checkpoints (every 500 lines)")
    print("  /IMPORTANT   - Find important messages")
    print("  /Thread-01   - Find messages from thread 1")


if __name__ == "__main__":
    main()
