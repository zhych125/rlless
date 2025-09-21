# Search Enhancements Stage 4 Benchmark Results

**Run details**
- Date: 2025-09-21 09:42:21 CST
- Commit: 56d0b7a62214bf754c04275f0c7b43ba04845558
- Commands:
  - `cargo bench --bench search_performance`
  - `cargo bench --bench file_access`

## Highlights
- Search pattern benches show notable wins for small literal lookups (≈11.7% faster) and regex searches on 256KB-32MB data (≈2.5-2.8% faster).
- Whole-word search regressed on larger fixtures (≈3.3-3.4% slower) and should be profiled alongside recent state-machine tweaks.
- File opening improved for 512KB plain files (≈5.1% faster) but regressed for 8MB workloads and gzip decoding of 512KB archives.
- Navigation `search_prev` path improved slightly (≈2.2%) while other navigation/search caching scenarios remained flat within noise.

## Full Results
| Suite | Scenario | Mean time | Change vs baseline | Status |
| --- | --- | --- | --- | --- |
| complex_regex | correlation_pattern/20MB | 4.069 ms | -1.87% | No significant change |
| complex_regex | correlation_pattern/5MB | 1.025 ms | -0.98% | No significant change |
| complex_regex | correlation_pattern/64MB | 12.990 ms | -0.54% | No significant change |
| complex_regex | ipv4_pattern/20MB | 4.334 ms | -1.19% | No significant change |
| complex_regex | ipv4_pattern/5MB | 1.082 ms | -1.12% | No significant change |
| complex_regex | ipv4_pattern/64MB | 13.777 ms | -9.30% | Improved |
| complex_regex | json_structure/20MB | 4.079 ms | -1.94% | No significant change |
| complex_regex | json_structure/5MB | 1.023 ms | -1.32% | No significant change |
| complex_regex | json_structure/64MB | 13.046 ms | -0.77% | No significant change |
| complex_regex | memory_alert/20MB | 4.074 ms | -1.77% | No significant change |
| complex_regex | memory_alert/5MB | 1.025 ms | -0.67% | No significant change |
| complex_regex | memory_alert/64MB | 13.017 ms | -0.13% | No significant change |
| complex_regex | session_pattern/20MB | 4.254 ms | -2.83% | Improved |
| complex_regex | session_pattern/5MB | 1.070 ms | -1.52% | No significant change |
| complex_regex | session_pattern/64MB | 13.601 ms | -1.17% | No significant change |
| file_opening | gzip/512KB | 470.630 us | +3.16% | Regressed |
| file_opening | gzip/64MB | 65.236 ms | +1.01% | No significant change |
| file_opening | gzip/8MB | 7.521 ms | +1.53% | No significant change |
| file_opening | plain/512KB | 51.791 us | -5.13% | Improved |
| file_opening | plain/64MB | 44.194 us | +1.29% | No significant change |
| file_opening | plain/8MB | 323.894 us | +4.35% | Regressed |
| line_access | gzip/2MB | 1.538 us | -0.15% | No significant change |
| line_access | gzip/64MB | 1.539 us | +0.45% | No significant change |
| line_access | plain/2MB | 1.543 us | +0.21% | No significant change |
| line_access | plain/64MB | 1.544 us | +0.01% | No significant change |
| random_start_search | backward_random/20MB | 1.126 us | -0.11% | No significant change |
| random_start_search | backward_random/5MB | 1.127 us | +0.15% | No significant change |
| random_start_search | backward_random/64MB | 1.295 us | -1.75% | No significant change |
| random_start_search | context_random/20MB | 1.099 us | +0.28% | No significant change |
| random_start_search | context_random/5MB | 1.091 us | +0.63% | No significant change |
| random_start_search | context_random/64MB | 1.215 us | -1.60% | No significant change |
| random_start_search | ipv4_random/20MB | 2.580 ms | -0.37% | No significant change |
| random_start_search | ipv4_random/5MB | 657.823 us | -0.23% | No significant change |
| random_start_search | ipv4_random/64MB | 8.083 ms | -0.12% | No significant change |
| random_start_search | literal_random/20MB | 1.159 us | +1.32% | No significant change |
| random_start_search | literal_random/5MB | 1.097 us | +1.79% | No significant change |
| random_start_search | literal_random/64MB | 1.229 us | -0.90% | No significant change |
| random_start_search | regex_random/20MB | 321.297 ns | -0.67% | No significant change |
| random_start_search | regex_random/5MB | 292.951 ns | +0.46% | No significant change |
| random_start_search | regex_random/64MB | 374.870 ns | +1.17% | No significant change |
| search_caching | cached_search | 169.163 ns | +0.06% | No significant change |
| search_caching | uncached_search | 87.234 us | +0.24% | No significant change |
| search_navigation | search_next | 893.785 ns | +0.08% | No significant change |
| search_navigation | search_prev | 511.032 ns | -2.17% | Improved |
| search_navigation | search_with_context | 3.296 ms | +0.48% | No significant change |
| search_patterns | case_insensitive/256KB | 229.454 ns | -1.91% | No significant change |
| search_patterns | case_insensitive/32MB | 230.104 ns | +0.99% | No significant change |
| search_patterns | case_insensitive/5MB | 231.427 ns | +0.82% | No significant change |
| search_patterns | literal_search/256KB | 170.253 ns | -11.72% | Improved |
| search_patterns | literal_search/32MB | 167.320 ns | -0.58% | No significant change |
| search_patterns | literal_search/5MB | 168.044 ns | -0.39% | No significant change |
| search_patterns | regex_search/256KB | 180.901 ns | -2.78% | Improved |
| search_patterns | regex_search/32MB | 179.790 ns | -2.48% | Improved |
| search_patterns | regex_search/5MB | 180.939 ns | +0.40% | No significant change |
| search_patterns | whole_word/256KB | 201.025 ns | -5.51% | Improved |
| search_patterns | whole_word/32MB | 206.068 ns | +3.43% | Regressed |
| search_patterns | whole_word/5MB | 208.971 ns | +3.30% | Regressed |
