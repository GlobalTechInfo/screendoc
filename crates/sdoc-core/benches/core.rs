//! Criterion benchmarks for the sdoc-core hot paths: secret masking, marker
//! demuxing, and cast parsing.

use std::hint::black_box;

use criterion::{Criterion, criterion_group, criterion_main};
use sdoc_core::markers::{encode_chapter_marker, encode_cmd_marker};
use sdoc_core::{CompiledRule, MarkerReader, compile_pattern, mask_rules, parse_cast_text};

fn bench_mask_rules(c: &mut Criterion) {
    let re = compile_pattern(black_box(r"(?i)api[_-]?key\s*[:=]\s*\S+")).unwrap();
    let rules = vec![CompiledRule { name: "bench".into(), re, replacement: "[REDACTED]".into() }];
    let input: String = (0..4096)
        .map(|i| format!("deploy {i}: API_KEY=sk-abcdef{i} password=hunter2 echo done\n"))
        .collect();
    c.bench_function("mask_rules/4k_lines", |b| b.iter(|| mask_rules(black_box(&input), &rules)));
}

fn build_marker_stream(n_cmds: u64) -> Vec<u8> {
    let mut stream = Vec::new();
    for i in 0..n_cmds {
        let cmd = format!("cargo test -- --exact case_{i}");
        stream.extend_from_slice(encode_cmd_marker(black_box(i), 0, &cmd).as_bytes());
        stream.extend_from_slice(b"\r\nsome output line 1\r\nsome output line 2\r\n");
        if i % 16 == 0 {
            stream.extend_from_slice(encode_chapter_marker(&format!("Chapter {i}")).as_bytes());
        }
    }
    stream
}

fn bench_marker_reader(c: &mut Criterion) {
    let stream = build_marker_stream(512);
    c.bench_function("marker_reader/demux_512_cmds", |b| {
        b.iter(|| {
            let mut reader = MarkerReader::new();
            let (clean, markers) = reader.feed(black_box(&stream));
            clean.len() + markers.len()
        })
    });
}

fn build_cast_text(n_lines: usize) -> String {
    let mut text = String::with_capacity(n_lines * 96);
    text.push_str("{\"version\": 2, \"terminal_width\": 120, \"terminal_height\": 30, \"timestamp\": 1700000000}\n");
    let mut t = 0.0;
    for i in 0..n_lines {
        t += 0.01;
        let ts = format!("{:.3}", t);
        let data = format!("line {i}: some terminal output");
        // `\\r\\n` keeps the CR/LF escaped inside the JSON string, so the
        // synthesized cast stays valid JSON for serde_json.
        text.push_str(&format!("[{}, \"o\", \"{}\\r\\n\"]\n", ts, data));
    }
    text
}

fn bench_parse_cast(c: &mut Criterion) {
    let text = build_cast_text(5000);
    c.bench_function("parse_cast/5k_lines", |b| {
        b.iter(|| parse_cast_text(black_box(&text)).map(|p| p.events.len()))
    });
}

criterion_group!(benches, bench_mask_rules, bench_marker_reader, bench_parse_cast);
criterion_main!(benches);
