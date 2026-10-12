//! Text wrapping with the ECM font metrics. Layout happens in the guest, so
//! the lines decided here are exactly what every renderer draws: one `text`
//! node per line (§6).

use ecm_ui_proto::fonts::Face;

/// Break `text` into lines no wider than `max_w` lu (None = no limit).
/// Hard line breaks (`\n`) always break. Lines break at spaces; a word wider
/// than a whole line is split between characters. The space a line breaks
/// at is dropped.
pub fn wrap(face: &Face, text: &str, max_w: Option<i32>) -> Vec<String> {
    let mut out = Vec::new();
    for para in text.split('\n') {
        let Some(max) = max_w else {
            out.push(para.to_string());
            continue;
        };
        let mut line = String::new();
        let mut line_w = 0;
        let space = face.advance(' ');
        for (i, word) in para.split(' ').enumerate() {
            let word_w = face.text_width(word);
            if i > 0 {
                if line_w + space + word_w <= max {
                    line.push(' ');
                    line.push_str(word);
                    line_w += space + word_w;
                    continue;
                }
                out.push(std::mem::take(&mut line));
                line_w = 0;
            }
            if word_w <= max {
                line.push_str(word);
                line_w = word_w;
                continue;
            }
            // Split an over-long word between characters.
            for c in word.chars() {
                let cw = face.advance(c);
                if line_w + cw > max && !line.is_empty() {
                    out.push(std::mem::take(&mut line));
                    line_w = 0;
                }
                line.push(c);
                line_w += cw;
            }
        }
        out.push(line);
    }
    out
}

/// Character index in `text` nearest to `x` lu from its start.
pub fn index_at(face: &Face, text: &str, x: i32) -> usize {
    let mut pos = 0;
    for (i, c) in text.chars().enumerate() {
        let w = face.advance(c);
        if x < pos + w / 2 {
            return i;
        }
        pos += w;
    }
    text.chars().count()
}

/// Width of the first `chars` characters of `text`.
pub fn prefix_width(face: &Face, text: &str, chars: usize) -> i32 {
    text.chars().take(chars).map(|c| face.advance(c)).sum()
}

#[cfg(test)]
mod tests {
    use super::*;
    use ecm_ui_proto::{fonts, Font};

    fn regular() -> Face {
        fonts::face(Font::Ui, 12).unwrap()
    }

    #[test]
    fn wraps_at_spaces_within_the_width() {
        let f = regular();
        let text = "The quick brown fox jumps over the lazy dog";
        let lines = wrap(&f, text, Some(60));
        assert!(lines.len() > 1);
        for l in &lines {
            assert!(f.text_width(l) <= 60, "{l:?} is {} wide", f.text_width(l));
        }
        assert_eq!(lines.join(" "), text, "only the break spaces are dropped");
        // A wider box takes more per line; no limit takes everything.
        assert!(wrap(&f, text, Some(120)).len() < lines.len());
        assert_eq!(wrap(&f, text, None), vec![text.to_string()]);
    }

    #[test]
    fn hard_breaks_and_long_words() {
        let f = regular();
        assert_eq!(wrap(&f, "a\nb", Some(500)), vec!["a", "b"]);
        let long = "W".repeat(20); // 6 lu each
        let lines = wrap(&f, &long, Some(30));
        assert_eq!(lines, vec!["WWWWW", "WWWWW", "WWWWW", "WWWWW"]);
        // The 2x size wraps twice as early.
        let big = fonts::face(Font::Ui, 24).unwrap();
        assert_eq!(wrap(&big, &long, Some(60)).len(), 4);
    }

    #[test]
    fn caret_positions() {
        let f = regular();
        assert_eq!(prefix_width(&f, "Hi", 2), 8);
        assert_eq!(index_at(&f, "Hi", 0), 0);
        assert_eq!(index_at(&f, "Hi", 4), 1);
        assert_eq!(index_at(&f, "Hi", 100), 2);
    }
}
