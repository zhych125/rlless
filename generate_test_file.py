#!/usr/bin/env python3
"""
Generate a large, readable text file for testing rlless.
Creates a file with realistic log-like content including:
- Timestamps
- Log levels (INFO, WARN, ERROR, DEBUG)
- Various message types
- Some patterns to search for
"""

import random
import datetime
import sys

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

def main():
    output_file = "large_test_file.log"
    target_size_mb = 1024  # Generate 25MB to ensure > 20MB

    print(f"Generating {target_size_mb}MB test file: {output_file}")
    print("This may take a few moments...")

    line_count = 0
    current_size = 0
    target_size = target_size_mb * 1024 * 1024  # Convert to bytes

    with open(output_file, 'w') as f:
        while current_size < target_size:
            line = generate_log_line(line_count) + '\n'
            f.write(line)
            current_size += len(line.encode('utf-8'))
            line_count += 1

            # Progress indicator
            if line_count % 10000 == 0:
                progress = (current_size / target_size) * 100
                print(f"  Progress: {progress:.1f}% ({current_size / 1024 / 1024:.1f}MB) - {line_count} lines", end='\r')

    # Final stats
    actual_size_mb = current_size / 1024 / 1024
    print(f"\nGenerated {output_file}:")
    print(f"  Size: {actual_size_mb:.2f}MB")
    print(f"  Lines: {line_count:,}")
    print(f"\nYou can now test with: ./target/debug/rlless {output_file}")
    print("\nSome interesting patterns to search for:")
    print("  /ERROR       - Find error messages")
    print("  /CRITICAL    - Find critical errors")
    print("  /MILESTONE   - Find milestone markers (every 100 lines)")
    print("  /CHECKPOINT  - Find checkpoints (every 500 lines)")
    print("  /IMPORTANT   - Find important messages")
    print("  /Thread-01   - Find messages from thread 1")

if __name__ == "__main__":
    main()
