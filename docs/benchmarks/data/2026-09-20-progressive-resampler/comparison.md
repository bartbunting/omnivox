# Voice benchmark comparison

Flagged series: 0; incomplete/low-sample series: 3.
Flag when p95 rises by more than both 5 ms and 15%. Minimum 20 samples.
This is a regression screen, not a statistical proof of unchanged performance.

| Route / path / mode / case / metric | Before p95 ms | After p95 ms | Change ms | Assessment |
|---|---:|---:|---:|---|
| dectalk/legacy/warm/character/dispatch_to_source_ms | 56.73 | 64.51 | +7.78 | within threshold |
| dectalk/legacy/warm/character/dispatch_to_terminal_ms | 57.04 | 64.71 | +7.67 | within threshold |
| dectalk/legacy/warm/character/source_to_terminal_ms | 0.50 | 0.44 | -0.06 | within threshold |
| dectalk/legacy/warm/concurrent_main/dispatch_to_source_ms | 34.83 | 32.02 | -2.81 | within threshold |
| dectalk/legacy/warm/concurrent_main/dispatch_to_terminal_ms | 56.16 | 58.30 | +2.14 | within threshold |
| dectalk/legacy/warm/concurrent_main/source_to_terminal_ms | 27.90 | 29.39 | +1.49 | within threshold |
| dectalk/legacy/warm/concurrent_notification/dispatch_to_source_ms | 34.20 | 32.34 | -1.86 | within threshold |
| dectalk/legacy/warm/concurrent_notification/dispatch_to_terminal_ms | 56.56 | 58.85 | +2.29 | within threshold |
| dectalk/legacy/warm/concurrent_notification/source_to_terminal_ms | 27.35 | 30.06 | +2.71 | within threshold |
| dectalk/legacy/warm/dense/dispatch_to_source_ms | 33.14 | 33.24 | +0.10 | within threshold |
| dectalk/legacy/warm/dense/dispatch_to_terminal_ms | 107.09 | 102.07 | -5.01 | within threshold |
| dectalk/legacy/warm/dense/source_to_terminal_ms | 75.85 | 75.27 | -0.58 | within threshold |
| dectalk/legacy/warm/line/dispatch_to_source_ms | 33.25 | 34.29 | +1.04 | within threshold |
| dectalk/legacy/warm/line/dispatch_to_terminal_ms | 59.41 | 56.92 | -2.49 | within threshold |
| dectalk/legacy/warm/line/source_to_terminal_ms | 27.28 | 31.84 | +4.56 | within threshold |
| dectalk/legacy/warm/multipart/dispatch_to_source_ms | 33.16 | 31.28 | -1.88 | within threshold |
| dectalk/legacy/warm/multipart/dispatch_to_terminal_ms | 58.90 | 51.18 | -7.72 | within threshold |
| dectalk/legacy/warm/multipart/source_to_terminal_ms | 29.39 | 26.77 | -2.62 | within threshold |
| dectalk/legacy/warm/replacement/cancel_terminal_ms | 0.55 | 0.60 | +0.04 | within threshold |
| dectalk/legacy/warm/replacement/dispatch_to_source_ms | 62.45 | 61.60 | -0.85 | within threshold |
| dectalk/legacy/warm/replacement/dispatch_to_terminal_ms | 134.44 | 134.12 | -0.32 | within threshold |
| dectalk/legacy/warm/replacement/source_to_terminal_ms | 78.28 | 78.71 | +0.43 | within threshold |
| dectalk/legacy/warm/server_ready/process_start_to_ready_ms | 723.17 | 752.15 | +28.98 | too few samples |
| dectalk/legacy/warm/word/dispatch_to_source_ms | 29.67 | 32.71 | +3.03 | within threshold |
| dectalk/legacy/warm/word/dispatch_to_terminal_ms | 53.23 | 59.49 | +6.26 | within threshold |
| dectalk/legacy/warm/word/source_to_terminal_ms | 26.84 | 28.99 | +2.15 | within threshold |
| eloquence/legacy/warm/character/dispatch_to_source_ms | 20.48 | 4.06 | -16.42 | within threshold |
| eloquence/legacy/warm/character/dispatch_to_terminal_ms | 20.89 | 4.39 | -16.49 | within threshold |
| eloquence/legacy/warm/character/source_to_terminal_ms | 0.46 | 0.36 | -0.11 | within threshold |
| eloquence/legacy/warm/concurrent_main/dispatch_to_source_ms | 21.14 | 7.62 | -13.52 | within threshold |
| eloquence/legacy/warm/concurrent_main/dispatch_to_terminal_ms | 22.69 | 12.42 | -10.28 | within threshold |
| eloquence/legacy/warm/concurrent_main/source_to_terminal_ms | 2.21 | 5.19 | +2.98 | within threshold |
| eloquence/legacy/warm/concurrent_notification/dispatch_to_source_ms | 21.06 | 8.13 | -12.93 | within threshold |
| eloquence/legacy/warm/concurrent_notification/dispatch_to_terminal_ms | 22.86 | 12.63 | -10.23 | within threshold |
| eloquence/legacy/warm/concurrent_notification/source_to_terminal_ms | 2.37 | 5.08 | +2.71 | within threshold |
| eloquence/legacy/warm/dense/dispatch_to_source_ms | 19.52 | 7.51 | -12.01 | within threshold |
| eloquence/legacy/warm/dense/dispatch_to_terminal_ms | 40.55 | 21.83 | -18.71 | within threshold |
| eloquence/legacy/warm/dense/source_to_terminal_ms | 21.93 | 14.94 | -7.00 | within threshold |
| eloquence/legacy/warm/line/dispatch_to_source_ms | 20.00 | 5.89 | -14.12 | within threshold |
| eloquence/legacy/warm/line/dispatch_to_terminal_ms | 22.01 | 9.72 | -12.29 | within threshold |
| eloquence/legacy/warm/line/source_to_terminal_ms | 2.07 | 4.47 | +2.40 | within threshold |
| eloquence/legacy/warm/multipart/dispatch_to_source_ms | 18.93 | 8.22 | -10.71 | within threshold |
| eloquence/legacy/warm/multipart/dispatch_to_terminal_ms | 20.95 | 15.07 | -5.88 | within threshold |
| eloquence/legacy/warm/multipart/source_to_terminal_ms | 2.43 | 7.38 | +4.95 | within threshold |
| eloquence/legacy/warm/replacement/cancel_terminal_ms | 0.59 | 0.53 | -0.06 | within threshold |
| eloquence/legacy/warm/replacement/dispatch_to_source_ms | 50.94 | 38.28 | -12.67 | within threshold |
| eloquence/legacy/warm/replacement/dispatch_to_terminal_ms | 70.57 | 49.68 | -20.89 | within threshold |
| eloquence/legacy/warm/replacement/source_to_terminal_ms | 23.10 | 16.40 | -6.70 | within threshold |
| eloquence/legacy/warm/server_ready/process_start_to_ready_ms | 684.89 | 2653.95 | +1969.06 | too few samples |
| eloquence/legacy/warm/word/dispatch_to_source_ms | 22.19 | 5.93 | -16.27 | within threshold |
| eloquence/legacy/warm/word/dispatch_to_terminal_ms | 22.55 | 5.99 | -16.56 | within threshold |
| eloquence/legacy/warm/word/source_to_terminal_ms | 0.49 | 0.37 | -0.11 | within threshold |
| espeak/legacy/warm/character/dispatch_to_source_ms | 19.09 | 3.19 | -15.90 | within threshold |
| espeak/legacy/warm/character/dispatch_to_terminal_ms | 21.37 | 4.54 | -16.83 | within threshold |
| espeak/legacy/warm/character/source_to_terminal_ms | 3.49 | 1.64 | -1.85 | within threshold |
| espeak/legacy/warm/concurrent_main/dispatch_to_source_ms | 20.65 | 3.90 | -16.75 | within threshold |
| espeak/legacy/warm/concurrent_main/dispatch_to_terminal_ms | 52.21 | 35.49 | -16.72 | within threshold |
| espeak/legacy/warm/concurrent_main/source_to_terminal_ms | 31.81 | 31.71 | -0.10 | within threshold |
| espeak/legacy/warm/concurrent_notification/dispatch_to_source_ms | 20.19 | 4.41 | -15.77 | within threshold |
| espeak/legacy/warm/concurrent_notification/dispatch_to_terminal_ms | 49.11 | 35.45 | -13.65 | within threshold |
| espeak/legacy/warm/concurrent_notification/source_to_terminal_ms | 31.36 | 32.10 | +0.74 | within threshold |
| espeak/legacy/warm/line/dispatch_to_source_ms | 19.14 | 3.63 | -15.51 | within threshold |
| espeak/legacy/warm/line/dispatch_to_terminal_ms | 51.08 | 33.35 | -17.73 | within threshold |
| espeak/legacy/warm/line/source_to_terminal_ms | 32.36 | 30.56 | -1.80 | within threshold |
| espeak/legacy/warm/multipart/dispatch_to_source_ms | 19.37 | 3.69 | -15.68 | within threshold |
| espeak/legacy/warm/multipart/dispatch_to_terminal_ms | 54.57 | 35.31 | -19.26 | within threshold |
| espeak/legacy/warm/multipart/source_to_terminal_ms | 36.64 | 32.20 | -4.44 | within threshold |
| espeak/legacy/warm/replacement/cancel_terminal_ms | 0.51 | 0.55 | +0.04 | within threshold |
| espeak/legacy/warm/replacement/dispatch_to_source_ms | 55.57 | 36.19 | -19.38 | within threshold |
| espeak/legacy/warm/replacement/dispatch_to_terminal_ms | 143.34 | 89.90 | -53.43 | within threshold |
| espeak/legacy/warm/replacement/source_to_terminal_ms | 93.26 | 63.75 | -29.51 | within threshold |
| espeak/legacy/warm/server_ready/process_start_to_ready_ms | 712.77 | 672.51 | -40.26 | too few samples |
| espeak/legacy/warm/word/dispatch_to_source_ms | 19.68 | 3.46 | -16.22 | within threshold |
| espeak/legacy/warm/word/dispatch_to_terminal_ms | 25.43 | 9.20 | -16.24 | within threshold |
| espeak/legacy/warm/word/source_to_terminal_ms | 6.81 | 6.10 | -0.71 | within threshold |
