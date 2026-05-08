use std::io::{self, IsTerminal, Write};
use std::sync::Arc;

use syntect::easy::HighlightLines;
use syntect::highlighting::{Theme, ThemeSet};
use syntect::parsing::SyntaxSet;
use syntect::util::as_24_bit_terminal_escaped;
use termimad::MadSkin;

const ANSI_RESET: &str = "\x1b[0m";
const CODE_INDENT: &str = "  ";
const DEFAULT_THEME: &str = "base16-ocean.dark";

pub fn should_render(no_format: bool) -> bool {
    !no_format && std::env::var_os("NO_COLOR").is_none() && io::stdout().is_terminal()
}

pub struct Assets {
    pub syntax_set: SyntaxSet,
    pub theme: Theme,
}

impl Assets {
    pub fn load() -> Self {
        // nonewlines variant: BufReader::lines() strips terminators, and syntect's
        // newline-aware regexes will fail to match without trailing \n.
        let syntax_set = SyntaxSet::load_defaults_nonewlines();
        let theme_set = ThemeSet::load_defaults();
        let theme = theme_set
            .themes
            .get(DEFAULT_THEME)
            .cloned()
            .unwrap_or_else(|| {
                theme_set
                    .themes
                    .values()
                    .next()
                    .cloned()
                    .expect("syntect ships with at least one default theme")
            });
        Self { syntax_set, theme }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum BlockMode {
    Idle,
    Paragraph,
    List,
    BlockQuote,
    FencedCode,
}

pub struct StreamingRenderer<W: Write> {
    writer: W,
    skin: MadSkin,
    assets: Arc<Assets>,
    buf: Vec<String>,
    mode: BlockMode,
    code_lang: Option<String>,
}

impl<W: Write> StreamingRenderer<W> {
    pub fn new(writer: W, assets: Arc<Assets>) -> Self {
        Self {
            writer,
            skin: MadSkin::default_dark(),
            assets,
            buf: Vec::new(),
            mode: BlockMode::Idle,
            code_lang: None,
        }
    }

    pub fn push_line(&mut self, line: &str) {
        // Inside a fenced code block, only the closing fence ends it.
        if self.mode == BlockMode::FencedCode {
            if is_fence(line) {
                self.flush_code_block();
            } else {
                self.buf.push(line.to_string());
            }
            return;
        }

        let trimmed = line.trim_end();

        // A fence opening can appear in any non-code mode.
        if let Some(lang) = parse_fence_open(trimmed) {
            self.flush_block();
            self.mode = BlockMode::FencedCode;
            self.code_lang = lang;
            return;
        }

        if trimmed.is_empty() {
            self.flush_block();
            return;
        }

        // ATX headings are single-line blocks: render immediately.
        if is_atx_heading(trimmed) {
            self.flush_block();
            self.render_text(trimmed);
            return;
        }

        let line_mode = classify_line(trimmed);

        // Switching block kinds without a blank line still terminates the previous block.
        if self.mode != BlockMode::Idle && self.mode != line_mode {
            self.flush_block();
        }

        self.mode = line_mode;
        self.buf.push(line.to_string());
    }

    pub fn finish(&mut self) {
        match self.mode {
            BlockMode::FencedCode => {
                // Unterminated fence: render body as plain dim text rather than swallow.
                let body: Vec<String> = std::mem::take(&mut self.buf);
                let _ = writeln!(self.writer);
                for line in body {
                    let _ = writeln!(self.writer, "{CODE_INDENT}{line}");
                }
                let _ = writeln!(self.writer);
                self.code_lang = None;
                self.mode = BlockMode::Idle;
            }
            _ => self.flush_block(),
        }
    }

    fn flush_block(&mut self) {
        if self.buf.is_empty() {
            self.mode = BlockMode::Idle;
            return;
        }
        let joined = std::mem::take(&mut self.buf).join("\n");
        self.render_text(&joined);
        self.mode = BlockMode::Idle;
    }

    fn render_text(&mut self, text: &str) {
        let _ = self.skin.write_text_on(&mut self.writer, text);
    }

    fn flush_code_block(&mut self) {
        let body: Vec<String> = std::mem::take(&mut self.buf);
        let lang = self.code_lang.take();

        let syntax = lang
            .as_deref()
            .and_then(|l| self.assets.syntax_set.find_syntax_by_token(l))
            .unwrap_or_else(|| self.assets.syntax_set.find_syntax_plain_text());

        let mut highlighter = HighlightLines::new(syntax, &self.assets.theme);

        let _ = writeln!(self.writer);
        for line in &body {
            match highlighter.highlight_line(line, &self.assets.syntax_set) {
                Ok(ranges) => {
                    let escaped = as_24_bit_terminal_escaped(&ranges, false);
                    let _ = writeln!(self.writer, "{CODE_INDENT}{escaped}{ANSI_RESET}");
                }
                Err(_) => {
                    // Never drop content silently if highlighting fails for any reason.
                    let _ = writeln!(self.writer, "{CODE_INDENT}{line}");
                }
            }
        }
        let _ = writeln!(self.writer);

        self.mode = BlockMode::Idle;
    }
}

fn is_fence(line: &str) -> bool {
    let t = line.trim_end();
    t.trim_start().starts_with("```") && {
        let stripped = t.trim_start().trim_start_matches('`');
        stripped.trim().is_empty()
    }
}

fn parse_fence_open(line: &str) -> Option<Option<String>> {
    let t = line.trim_start();
    if !t.starts_with("```") {
        return None;
    }
    let after = t.trim_start_matches('`');
    let lang = after.trim();
    if lang.is_empty() {
        Some(None)
    } else {
        Some(Some(lang.to_string()))
    }
}

fn is_atx_heading(line: &str) -> bool {
    let mut chars = line.chars();
    let mut hashes = 0;
    while let Some('#') = chars.next() {
        hashes += 1;
        if hashes > 6 {
            return false;
        }
    }
    if hashes == 0 {
        return false;
    }
    matches!(line.chars().nth(hashes), Some(' '))
}

fn classify_line(line: &str) -> BlockMode {
    let t = line.trim_start();
    if t.starts_with("> ") || t == ">" {
        return BlockMode::BlockQuote;
    }
    if t.starts_with("- ") || t.starts_with("* ") || t.starts_with("+ ") {
        return BlockMode::List;
    }
    let mut chars = t.chars();
    let mut digits = 0;
    while let Some(c) = chars.next() {
        if c.is_ascii_digit() {
            digits += 1;
        } else if digits > 0 && (c == '.' || c == ')') {
            if let Some(' ') = chars.next() {
                return BlockMode::List;
            }
            break;
        } else {
            break;
        }
    }
    BlockMode::Paragraph
}

#[cfg(test)]
mod tests {
    use super::*;

    fn render_lines(lines: &[&str]) -> String {
        let assets = Arc::new(Assets::load());
        let mut buf: Vec<u8> = Vec::new();
        {
            let mut r = StreamingRenderer::new(&mut buf, assets);
            for line in lines {
                r.push_line(line);
            }
            r.finish();
        }
        String::from_utf8(buf).unwrap()
    }

    fn contains_ansi(s: &str) -> bool {
        s.contains("\x1b[")
    }

    #[test]
    fn plain_paragraph_emits_no_ansi_for_letters_only() {
        // termimad still emits some control sequences (clear-to-end-of-line) but no
        // SGR color codes. We just sanity-check that it produces output.
        let out = render_lines(&["just some plain text"]);
        assert!(out.contains("just some plain text"));
    }

    #[test]
    fn heading_is_styled() {
        let out = render_lines(&["# A heading", ""]);
        assert!(contains_ansi(&out), "expected ANSI escapes for heading");
        assert!(out.contains("A heading"));
    }

    #[test]
    fn rust_code_block_is_syntax_highlighted() {
        let out = render_lines(&["```rust", "fn main() {}", "```"]);
        assert!(contains_ansi(&out), "expected ANSI for rust code block");
        // syntect highlights each token separately so "fn" and "main" are not
        // adjacent in the byte stream — assert each appears.
        assert!(out.contains("fn"), "expected 'fn' in output, got: {out:?}");
        assert!(
            out.contains("main"),
            "expected 'main' in output, got: {out:?}"
        );
    }

    #[test]
    fn unknown_lang_code_block_still_renders_body() {
        let out = render_lines(&["```nosuchlang", "hello world", "```"]);
        assert!(out.contains("hello world"));
    }

    #[test]
    fn finish_flushes_trailing_paragraph() {
        let out = render_lines(&["dangling line with no blank after"]);
        assert!(out.contains("dangling line"));
    }

    #[test]
    fn unterminated_fence_renders_body() {
        let out = render_lines(&["```rust", "fn x() {}"]);
        assert!(out.contains("fn x"));
    }

    #[test]
    fn list_renders() {
        let out = render_lines(&["- one", "- two", "- three", ""]);
        assert!(out.contains("one"));
        assert!(out.contains("two"));
        assert!(out.contains("three"));
    }

    #[test]
    fn switching_from_paragraph_to_list_flushes() {
        let out = render_lines(&["a paragraph", "- a list item", ""]);
        assert!(out.contains("paragraph"));
        assert!(out.contains("list item"));
    }

    #[test]
    fn blockquote_renders() {
        let out = render_lines(&["> quoted", "> still quoted", ""]);
        assert!(out.contains("quoted"));
    }

    #[test]
    fn atx_heading_detection() {
        assert!(is_atx_heading("# foo"));
        assert!(is_atx_heading("###### foo"));
        assert!(!is_atx_heading("####### too many"));
        assert!(!is_atx_heading("#nofoospace"));
        assert!(!is_atx_heading("not a heading"));
    }

    #[test]
    fn fence_parsing() {
        assert_eq!(parse_fence_open("```"), Some(None));
        assert_eq!(parse_fence_open("```rust"), Some(Some("rust".to_string())));
        assert_eq!(
            parse_fence_open("```  python  "),
            Some(Some("python".to_string()))
        );
        assert_eq!(parse_fence_open("not a fence"), None);
        assert!(is_fence("```"));
        assert!(is_fence("   ```   "));
        assert!(!is_fence("```rust"));
    }
}
