use vte::{Params, Parser, Perform};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Color {
    Indexed(u8),
    Rgb(u8, u8, u8),
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Style {
    pub fg: Option<Color>,
    pub bg: Option<Color>,
    pub bold: bool,
}

/// Byte range of `StyledLine::text` drawn with a non-default style.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Span {
    pub start: usize,
    pub end: usize,
    pub style: Style,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct StyledLine {
    pub text: String,
    pub spans: Vec<Span>,
}

impl StyledLine {
    pub fn plain(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            spans: Vec::new(),
        }
    }
}

#[derive(Default)]
pub struct Feed {
    pub lines: Vec<StyledLine>,
    /// Bytes the terminal must answer with (cursor position reports).
    pub reply: Vec<u8>,
}

pub struct LineProcessor {
    parser: Parser,
    state: LineState,
}

impl Default for LineProcessor {
    fn default() -> Self {
        Self::new()
    }
}

impl LineProcessor {
    pub fn new() -> Self {
        Self {
            parser: Parser::new(),
            state: LineState::default(),
        }
    }

    pub fn feed(&mut self, bytes: &[u8]) -> Feed {
        self.parser.advance(&mut self.state, bytes);
        Feed {
            lines: std::mem::take(&mut self.state.done),
            reply: std::mem::take(&mut self.state.reply),
        }
    }

    pub fn finish(&mut self) -> Option<StyledLine> {
        let line = self.state.take_line();
        (!line.text.is_empty()).then_some(line)
    }
}

#[derive(Default)]
struct LineState {
    cells: Vec<(char, Style)>,
    cursor: usize,
    style: Style,
    done: Vec<StyledLine>,
    reply: Vec<u8>,
}

impl LineState {
    fn put(&mut self, c: char) {
        while self.cells.len() < self.cursor {
            self.cells.push((' ', Style::default()));
        }
        if self.cursor < self.cells.len() {
            self.cells[self.cursor] = (c, self.style);
        } else {
            self.cells.push((c, self.style));
        }
        self.cursor += 1;
    }

    fn take_line(&mut self) -> StyledLine {
        let mut cells = std::mem::take(&mut self.cells);
        self.cursor = 0;
        while matches!(cells.last(), Some((c, s)) if c.is_whitespace() && s.bg.is_none()) {
            cells.pop();
        }
        let mut text = String::with_capacity(cells.len());
        let mut spans: Vec<Span> = Vec::new();
        for (c, style) in cells {
            let start = text.len();
            text.push(c);
            if style == Style::default() {
                continue;
            }
            match spans.last_mut() {
                Some(last) if last.end == start && last.style == style => last.end = text.len(),
                _ => spans.push(Span {
                    start,
                    end: text.len(),
                    style,
                }),
            }
        }
        StyledLine { text, spans }
    }

    fn erase_line(&mut self, mode: u16) {
        match mode {
            0 => self.cells.truncate(self.cursor),
            1 => {
                let end = (self.cursor + 1).min(self.cells.len());
                for cell in &mut self.cells[..end] {
                    *cell = (' ', Style::default());
                }
            }
            _ => self.cells.clear(),
        }
    }

    fn apply_sgr(&mut self, params: &Params) {
        let mut it = params.iter().map(|p| p[0]);
        if params.is_empty() {
            self.style = Style::default();
            return;
        }
        while let Some(p) = it.next() {
            match p {
                0 => self.style = Style::default(),
                1 => self.style.bold = true,
                22 => self.style.bold = false,
                30..=37 => self.style.fg = Some(Color::Indexed((p - 30) as u8)),
                39 => self.style.fg = None,
                40..=47 => self.style.bg = Some(Color::Indexed((p - 40) as u8)),
                49 => self.style.bg = None,
                90..=97 => self.style.fg = Some(Color::Indexed((p - 90 + 8) as u8)),
                100..=107 => self.style.bg = Some(Color::Indexed((p - 100 + 8) as u8)),
                38 | 48 => {
                    let color = match it.next() {
                        Some(5) => it.next().map(|n| Color::Indexed(n as u8)),
                        Some(2) => match (it.next(), it.next(), it.next()) {
                            (Some(r), Some(g), Some(b)) => {
                                Some(Color::Rgb(r as u8, g as u8, b as u8))
                            }
                            _ => None,
                        },
                        _ => None,
                    };
                    if p == 38 {
                        self.style.fg = color;
                    } else {
                        self.style.bg = color;
                    }
                }
                _ => {}
            }
        }
    }
}

impl Perform for LineState {
    fn print(&mut self, c: char) {
        self.put(c);
    }

    fn execute(&mut self, byte: u8) {
        match byte {
            b'\n' => {
                let line = self.take_line();
                self.done.push(line);
            }
            b'\r' => self.cursor = 0,
            0x08 => self.cursor = self.cursor.saturating_sub(1),
            b'\t' => {
                let next = (self.cursor / 8 + 1) * 8;
                while self.cursor < next {
                    self.put(' ');
                }
            }
            _ => {}
        }
    }

    fn csi_dispatch(&mut self, params: &Params, intermediates: &[u8], ignore: bool, action: char) {
        if ignore || !intermediates.is_empty() {
            return;
        }
        let first = params.iter().next().map(|p| p[0]).unwrap_or(0);
        match action {
            'm' => self.apply_sgr(params),
            'K' => self.erase_line(first),
            // ConPTY renders runs of spaces as cursor-forward / column moves within a line.
            'C' => self.cursor += first.max(1) as usize,
            'G' => self.cursor = first.max(1) as usize - 1,
            'X' => {
                let end = (self.cursor + first.max(1) as usize).min(self.cells.len());
                for cell in &mut self.cells[self.cursor.min(end)..end] {
                    *cell = (' ', Style::default());
                }
            }
            'n' if first == 6 => self.reply.extend_from_slice(b"\x1b[1;1R"),
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(input: &[u8]) -> Vec<String> {
        let mut p = LineProcessor::new();
        p.feed(input).lines.into_iter().map(|l| l.text).collect()
    }

    #[test]
    fn carriage_return_overwrites() {
        assert_eq!(
            lines(b"progress 0%\rprogress 50%\rprogress 100%\r\n"),
            ["progress 100%"]
        );
    }

    #[test]
    fn backspace_and_erase_line() {
        assert_eq!(lines(b"abcd\x08\x08X\n"), ["abXd"]);
        assert_eq!(lines(b"hello world\r\x1b[Kbye\n"), ["bye"]);
        assert_eq!(lines(b"hello world\x1b[6D\x1b[K\n"), ["hello world"]);
    }

    #[test]
    fn sgr_becomes_spans_and_is_stripped_from_text() {
        let mut p = LineProcessor::new();
        let out = p.feed(b"\x1b[31mred\x1b[0m plain\n");
        assert_eq!(out.lines[0].text, "red plain");
        assert_eq!(
            out.lines[0].spans,
            [Span {
                start: 0,
                end: 3,
                style: Style {
                    fg: Some(Color::Indexed(1)),
                    ..Style::default()
                }
            }]
        );
    }

    #[test]
    fn cursor_movement_and_modes_ignored() {
        assert_eq!(
            lines(b"\x1b[?25l\x1b[2J\x1b[Hready\x1b[?25h\r\n"),
            ["ready"]
        );
    }

    #[test]
    fn cursor_forward_inserts_spaces() {
        assert_eq!(lines(b"a\x1b[3Cb\n"), ["a   b"]);
    }

    #[test]
    fn cursor_position_request_is_answered() {
        let mut p = LineProcessor::new();
        assert_eq!(p.feed(b"\x1b[6n").reply, b"\x1b[1;1R");
    }

    #[test]
    fn partial_line_split_across_feeds() {
        let mut p = LineProcessor::new();
        assert!(p.feed("한글 ".as_bytes()[..4].as_ref()).lines.is_empty());
        let out = p.feed(&"한글 ".as_bytes()[4..]);
        assert!(out.lines.is_empty());
        assert_eq!(p.feed(b"ok\n").lines[0].text, "한글 ok");
    }
}
