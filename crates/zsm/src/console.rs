use std::collections::VecDeque;

use zsm_core::buffer::{Line, LineKind};
use zsm_core::output::Color;
use zsm_core::supervisor::Service;
use zsm_core::win::RichEdit;

pub const BACKGROUND: u32 = rgb(12, 12, 12);
const FOREGROUND: u32 = rgb(204, 204, 204);
const SYSTEM: u32 = rgb(97, 175, 239);
const INPUT: u32 = rgb(229, 192, 123);

const fn rgb(r: u8, g: u8, b: u8) -> u32 {
    r as u32 | (g as u32) << 8 | (b as u32) << 16
}

const PALETTE: [u32; 16] = [
    rgb(12, 12, 12),
    rgb(197, 15, 31),
    rgb(19, 161, 14),
    rgb(193, 156, 0),
    rgb(0, 55, 218),
    rgb(136, 23, 152),
    rgb(58, 150, 221),
    rgb(204, 204, 204),
    rgb(118, 118, 118),
    rgb(231, 72, 86),
    rgb(22, 198, 12),
    rgb(249, 241, 165),
    rgb(59, 120, 255),
    rgb(180, 0, 158),
    rgb(97, 214, 214),
    rgb(242, 242, 242),
];

fn color_ref(c: Color) -> u32 {
    match c {
        Color::Indexed(i) if i < 16 => PALETTE[i as usize],
        Color::Indexed(i) if i >= 232 => {
            let v = 8 + (i - 232) * 10;
            rgb(v, v, v)
        }
        Color::Indexed(i) => {
            let i = i - 16;
            let level = |n: u8| if n == 0 { 0 } else { 55 + n * 40 };
            rgb(level(i / 36), level(i / 6 % 6), level(i % 6))
        }
        Color::Rgb(r, g, b) => rgb(r, g, b),
    }
}

/// Incremental view of one service's scrollback in a RichEdit control.
pub struct ConsoleView {
    edit: RichEdit,
    service: Option<String>,
    last_seq: u64,
    /// UTF-16 length of each displayed line, oldest first.
    lengths: VecDeque<usize>,
    capacity: usize,
}

impl ConsoleView {
    pub fn new(edit: RichEdit, capacity: usize) -> Self {
        edit.set_background(BACKGROUND);
        Self {
            edit,
            service: None,
            last_seq: 0,
            lengths: VecDeque::new(),
            capacity,
        }
    }

    pub fn edit(&self) -> RichEdit {
        self.edit
    }

    pub fn set_capacity(&mut self, capacity: usize) {
        self.capacity = capacity.max(1);
    }

    /// Switches to `service` (or nothing), redrawing from its scrollback when it changes.
    pub fn show(&mut self, service: Option<&Service>, clear: impl FnOnce()) {
        let id = service.map(|s| s.id().to_owned());
        if id == self.service {
            return;
        }
        clear();
        self.service = id;
        self.last_seq = 0;
        self.lengths.clear();
        if let Some(svc) = service {
            self.pull(svc, || {});
        }
    }

    pub fn pull(&mut self, service: &Service, clear: impl FnOnce()) {
        let lines = service.lines_since(self.last_seq);
        let Some(first) = lines.first() else { return };
        if self.last_seq != 0 && first.seq > self.last_seq + 1 {
            clear();
            self.lengths.clear();
        }
        self.last_seq = lines.last().map_or(self.last_seq, |l| l.seq);
        let keep = lines.len().min(self.capacity);
        let lines = &lines[lines.len() - keep..];
        let mut chunks: Vec<(String, u32, bool)> = Vec::new();
        for line in lines {
            let newline = !self.lengths.is_empty() || !chunks.is_empty();
            push_line(&mut chunks, line, newline);
            self.lengths.push_back(line.text.encode_utf16().count());
        }
        let refs: Vec<(&str, u32, bool)> = chunks
            .iter()
            .map(|(t, c, b)| (t.as_str(), *c, *b))
            .collect();
        self.edit.append(&refs);
        let excess = self.lengths.len().saturating_sub(self.capacity);
        let chars: usize = self.lengths.drain(..excess).map(|n| n + 1).sum();
        self.edit.remove_prefix(chars);
    }
}

fn push_line(chunks: &mut Vec<(String, u32, bool)>, line: &Line, newline_before: bool) {
    if newline_before {
        chunks.push(("\r\n".into(), FOREGROUND, false));
    }
    let base = match line.kind {
        LineKind::Output => FOREGROUND,
        LineKind::System => SYSTEM,
        LineKind::Input => INPUT,
    };
    let mut pos = 0;
    for span in &line.spans {
        if span.start > pos {
            chunks.push((line.text[pos..span.start].into(), base, false));
        }
        let color = span.style.fg.map_or(base, color_ref);
        chunks.push((
            line.text[span.start..span.end].into(),
            color,
            span.style.bold,
        ));
        pos = span.end;
    }
    if pos < line.text.len() || line.text.is_empty() {
        chunks.push((line.text[pos..].into(), base, false));
    }
}
