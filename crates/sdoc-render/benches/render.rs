//! Criterion benchmarks for sdoc-render: cast parsing and idle compression.

use std::hint::black_box;

use criterion::{Criterion, criterion_group, criterion_main};
use sdoc_render::compress_idle;
use sdoc_render::frame::parse_cast;

fn build_cast_text(n_lines: usize) -> String {
    let mut text = String::with_capacity(n_lines * 96);
    text.push_str("{\"version\": 2, \"terminal_width\": 120, \"terminal_height\": 30, \"timestamp\": 1700000000}\n");
    let mut t = 0.0;
    for i in 0..n_lines {
        t += 0.01;
        let ts = format!("{:.3}", t);
        let data = format!("line {i}: output");
        text.push_str(&format!("[{}, \"o\", \"{}\\r\\n\"]\n", ts, data));
    }
    text
}

fn bench_parse_cast(c: &mut Criterion) {
    let text = build_cast_text(5000);
    c.bench_function("parse_cast/5k_lines", |b| {
        b.iter(|| parse_cast(black_box(&text)).map(|evs| evs.len()))
    });
}

fn bench_compress_idle(c: &mut Criterion) {
    let mut times = Vec::new();
    let mut t = 0.0;
    for i in 0..20_000 {
        t += 0.05;
        if i % 500 == 0 {
            t += 5.0; // idle gap above the default 2.0s limit
        }
        times.push(t);
    }
    c.bench_function("compress_idle/20k_events", |b| {
        b.iter(|| compress_idle(black_box(&times), 2.0).len())
    });
}

criterion_group!(benches, bench_parse_cast, bench_compress_idle);
criterion_main!(benches);
