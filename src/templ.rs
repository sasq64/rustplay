use anyhow::Result;
use regex::Regex;
use std::borrow::Borrow;
use std::collections::HashMap;
use std::fmt::Display;
use std::hash::Hash;
use std::ops::Deref;

#[derive(Clone, Debug, Default)]
pub struct PlaceHolder {
    start: usize,
    end: usize,
    pub col: usize,
    pub len: usize,
    pub line: usize,
    pub color: u32,
}

#[derive(Clone, Debug, Default)]
pub struct ColorString(Vec<(char, u32, u32)>);

impl ColorString {
    pub fn new(s: &str) -> Self {
        Self(s.chars().map(|c| (c, 0xff_ff_ff, 0)).collect())
    }

    pub fn as_string(&self) -> String {
        self.0.iter().map(|(c, _, _)| c).collect()
    }

    /// Convert a byte offset (from the string representation) to a char index.
    fn byte_to_char(&self, byte_offset: usize) -> usize {
        let mut bytes = 0;
        for (i, (c, _, _)) in self.0.iter().enumerate() {
            if bytes >= byte_offset {
                return i;
            }
            bytes += c.len_utf8();
        }
        self.0.len()
    }

    /// Replace a range specified by byte offsets (matching the string representation)
    /// with new text using default colors.
    pub fn replace_range_bytes(&mut self, byte_range: std::ops::Range<usize>, replacement: &str) {
        let start = self.byte_to_char(byte_range.start);
        let end = self.byte_to_char(byte_range.end);
        let new_chars: Vec<(char, u32, u32)> =
            replacement.chars().map(|c| (c, 0xff_ff_ff, 0)).collect();
        self.0.splice(start..end, new_chars);
    }

    pub fn extend_spaces(&mut self, n: usize) {
        self.0.extend(std::iter::repeat_n((' ', 0xff_ff_ff, 0), n));
    }
}

impl Deref for ColorString {
    type Target = Vec<(char, u32, u32)>;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl Display for ColorString {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        for (c, _, _) in &self.0 {
            write!(f, "{c}")?;
        }
        Ok(())
    }
}

#[derive(Clone, Debug)]
pub struct Template {
    raw_lines: Vec<ColorString>,
    templ: Vec<ColorString>,
    data: HashMap<String, PlaceHolder>,
    re: Regex,
}

fn dup_lines(dup_indexes: &[usize], lines: &mut Vec<ColorString>, h: usize) {
    // Duplicate lines until we reach target height
    if !dup_indexes.is_empty() {
        let s = (h - lines.len()) as f32 / dup_indexes.len() as f32;
        let mut n = 0;
        let mut f = 0.0;
        for i in dup_indexes.iter().rev() {
            f += s;
            while (n as f32) < f {
                lines.insert(*i, lines[*i].clone());
                n += 1;
            }
        }
    }
}

impl Template {
    fn as_string(&self) -> String {
        self.templ
            .iter()
            .map(|cs| cs.as_string())
            .collect::<Vec<_>>()
            .join("\n")
    }

    pub fn height(&self) -> usize {
        self.templ.len()
    }

    pub fn lines(&self) -> Vec<String> {
        self.templ.iter().map(|cs| cs.as_string()).collect()
    }

    pub fn color_lines(&self) -> &Vec<ColorString> {
        &self.templ
    }

    pub fn place_holders(&self) -> impl Iterator<Item = (&String, &PlaceHolder)> {
        self.data.iter()
    }

    pub fn get_pos(&self, id: &str) -> Option<(u16, u16)> {
        if let Some(ph) = self.data.get(id) {
            return Some((ph.col as u16, ph.line as u16));
        }
        None
    }

    fn render<T: Display, Q: Hash + Eq + Borrow<str>>(&self, data: &HashMap<Q, T>) -> Vec<String> {
        let mut result: Vec<String> = self.templ.iter().map(|cs| cs.as_string()).collect();
        for (key, val) in data {
            if let Some(ph) = self.data.get(key.borrow()) {
                let line = &mut result[ph.line];
                let text = format!("{val}");
                let mut end = ph.start + text.len();
                if end > line.len() {
                    end = line.len();
                }

                line.replace_range(ph.start..end, &text);
            }
        }
        result
    }

    fn render_string<T: Display, Q: Hash + Eq + Borrow<str>>(
        &self,
        data: &HashMap<Q, T>,
    ) -> String {
        let result = self.render(data);
        result.join("\n")
    }

    /// Template contains text or special patterns that are replaced;
    ///
    /// `$>` Pattern removed, next character is repeated until current line
    /// length = target length
    ///
    /// `$^` Pattern replaced with spaces, current line becomes part of
    /// vertical resize and may be duplicated any number of times.
    ///
    /// `$<symbol>` Pattern first replaced with spaces then with value from
    /// hashmap
    ///
    pub fn new(templ: &str, w: usize, h: usize) -> Result<Template> {
        let raw_lines: Vec<ColorString> =
            templ.lines().map(|line| ColorString::new(line)).collect();

        let re = Regex::new(r"\$(((?<var>\w+)\s*)|>(?<char>.)|(?<fill>\^))")?;

        let mut template = Template {
            raw_lines,
            templ: Vec::new(),
            data: HashMap::new(),
            re,
        };

        template.draw(w, h);
        Ok(template)
    }

    pub fn draw(&mut self, w: usize, h: usize) {
        let maxl = w;

        let mut data = HashMap::<String, PlaceHolder>::new();
        let mut dup_indexes = Vec::new();

        let max_len = self.raw_lines.iter().map(|l| l.len()).max().unwrap();

        // Find fill patterns ($> and $^), resize vertically and prepare for horizontal
        // Captures: var = var_name, char = char to repeat for '$>',
        // fill = '^' for line dup
        let mut lines: Vec<ColorString> = self
            .raw_lines
            .iter()
            .enumerate()
            .map(|(i, line)| {
                let mut target = line.clone();
                let line_str = line.as_string();
                for cap in self.re.captures_iter(&line_str) {
                    let m = cap.get(0).unwrap();
                    if let Some(x) = cap.name("char") {
                        let mut target_len = target.len();
                        if target_len < max_len {
                            let n = max_len - target_len;
                            target.extend_spaces(n);
                            target_len = max_len;
                        }
                        if w > target_len {
                            let len = (w - target_len) + 3;
                            let r = x.as_str().repeat(len);
                            target.replace_range_bytes(m.start()..m.end(), &r);
                        } else {
                            let r = x.as_str().repeat(2);
                            target.replace_range_bytes(m.start()..m.end(), &r);
                        }
                    }
                    if cap.name("fill").is_some() {
                        dup_indexes.push(i);
                        let spaces = " ".repeat(m.end() - m.start());
                        target.replace_range_bytes(m.start()..m.end(), &spaces);
                    }
                }
                let count = target.len();
                if count < maxl {
                    target.extend_spaces(maxl - count);
                }
                target
            })
            .collect();

        let h = if h > lines.len() { h } else { lines.len() };
        // Duplicate lines until we reach target height
        dup_lines(&dup_indexes, &mut lines, h);

        for (i, line) in lines.iter_mut().enumerate() {
            let mut clears = Vec::new();
            let line_str = line.as_string();
            for cap in self.re.captures_iter(&line_str) {
                let m = cap.get(0).unwrap();
                if let Some(x) = cap.name("var") {
                    let color: u32 = 0xff_ff_ff;
                    let col = line_str[..m.start()].chars().count();
                    let len = line_str[m.start()..m.end()].chars().count();
                    data.insert(
                        x.as_str().into(),
                        PlaceHolder {
                            start: m.start(),
                            end: m.end(),
                            col,
                            len,
                            line: i,
                            color,
                        },
                    );
                    clears.push((m.start(), x.end()));
                }
            }
            for (start, end) in clears {
                let spaces = " ".repeat(end - start);
                line.replace_range_bytes(start..end, &spaces);
            }
        }

        self.templ = lines;
        self.data = data;
    }

    pub(crate) fn get_placeholder(&self, key: &str) -> Option<&PlaceHolder> {
        self.data.get(key)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::Template;
    use crate::value::Value;
    use std::collections::HashMap;

    fn compare(a: &str, b: &str) -> bool {
        let one = a.lines().map(str::trim).filter(|l| !l.is_empty());
        let two = b.lines().map(str::trim).filter(|l| !l.is_empty());
        for (left, right) in one.zip(two) {
            if left != right {
                println!("'{left}' != '{right}'");
                return false;
            }
        }
        true
    }

    #[test]
    fn template_works() {
        let result = Template::new("Line $one\nX $x!\n---$>--", 10, 3).unwrap();
        let text = result.as_string();
        assert!(compare(&text, "Line\nX   !\n----------"));

        assert!(result.data["one"].start == 5);

        let result = Template::new(
            r#"
@pooh=asda
+----+$>---------+
|$^  | $hello $> |
+----+--------$>-+
|$^  |$>         |
+--=-+--$>------+
@name=gargamel
"#,
            20,
            5,
        )
        .unwrap();

        let text = result.render_string(&HashMap::from([("hello", "DOG!")]));
        assert!(compare(
            &text,
            r#"
+----+-------------+
|    | DOG!        |
+----+-------------+
|    |             |
+--=-+-------------+"#
        ));

        let text = result.render_string(&HashMap::from([("hello", "a much longer string")]));
        assert!(compare(
            &text,
            r#"
+----+-------------+
|    | a much longer string
+----+-------------+
|    |             |
+--=-+-------------+"#
        ));
    }

    #[test]
    fn player_templ_works() {
        let mut song_meta = HashMap::<String, Value>::new();
        song_meta.insert(
            "full_title".to_string(),
            Value::Text("Enigma (Musiklinjen)".to_string()),
        );
        song_meta.insert("isong".to_string(), Value::Number(2.0));
        song_meta.insert("len".to_string(), Value::Number(100.0));
        song_meta.insert("xxx".to_string(), Value::Data(Vec::<u8>::new()));

        let templ = Template::new(include_str!("../screen.templ"), 80, 10).unwrap();
        let x = templ.render_string(&song_meta);

        assert!(x.chars().count() > 400);
    }
}
