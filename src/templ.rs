use anyhow::Result;
use regex::Regex;
use std::borrow::Borrow;
use std::collections::HashMap;
use std::fmt::Display;
use std::hash::Hash;

#[derive(Clone, Debug, Default)]
pub struct PlaceHolder {
    start: usize,
    end: usize,
    pub col: usize,
    pub len: usize,
    pub line: usize,
    pub color: u32,
}

#[derive(Clone, Debug)]
pub struct Template {
    raw_lines: Vec<String>,
    templ: Vec<String>,
    data: HashMap<String, PlaceHolder>,
    re: Regex,
    renames: HashMap<String, String>,
    colors: HashMap<String, u32>,
}

fn dup_lines(dup_indexes: &[usize], lines: &mut Vec<String>, h: usize) {
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
        self.templ.join("\n")
    }

    pub fn height(&self) -> usize {
        self.templ.len()
    }

    pub fn lines(&self) -> &Vec<String> {
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
        let mut result = self.templ.clone();
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
    /// Special lines
    ///
    /// Variable alias `@short_symbol = real_symbol`
    ///
    pub fn new(templ: &str, w: usize, h: usize) -> Result<Template> {
        let alias_re = Regex::new(r"\@(\w+)=(\w+)?(:#([a-fA-F0-9]+))?")?;

        let mut renames: HashMap<String, String> = HashMap::new();
        let mut colors: HashMap<String, u32> = HashMap::new();

        // Strip alias assignments from template
        let raw_lines: Vec<String> = templ
            .lines()
            .filter(|line| {
                if let Some(m) = alias_re.captures(line) {
                    if let Some(var) = m.get(1) {
                        if let Some(alias) = m.get(2) {
                            renames.insert(alias.as_str().to_string(), var.as_str().to_string());
                        }
                        if let Some(color) = m.get(4)
                            && let Ok(rgb) = u32::from_str_radix(color.as_str(), 16)
                        {
                            colors.insert(var.as_str().to_string(), rgb);
                        }
                    }
                    return false;
                }
                true
            })
            .map(|line| line.to_string())
            .collect();

        let re = Regex::new(r"\$(((?<var>\w+)\s*)|>(?<char>.)|(?<fill>\^))")?;

        let mut template = Template {
            raw_lines,
            templ: Vec::new(),
            data: HashMap::new(),
            re,
            renames,
            colors,
        };

        template.draw(w, h);
        Ok(template)
    }

    pub fn draw(&mut self, w: usize, h: usize) {
        let spaces = "                                                                   ";
        let maxl = w;

        let mut data = HashMap::<String, PlaceHolder>::new();
        let mut dup_indexes = Vec::new();

        // Find fill patterns ($> and $^), resize vertically and prepare for horizontal
        // Captures: var = var_name, char = char to repeat for '$>',
        // fill = '^' for line dup
        let mut lines: Vec<String> = self
            .raw_lines
            .iter()
            .enumerate()
            .map(|(i, line)| {
                let mut target = line.to_string();
                for cap in self.re.captures_iter(line) {
                    let m = cap.get(0).unwrap();
                    if let Some(x) = cap.name("char") {
                        let target_len = target.chars().count();
                        if w > target_len {
                            let len = (w - target_len) + 3;
                            let r = x.as_str().repeat(len);
                            target.replace_range(m.start()..m.end(), &r);
                        } else {
                            let r = x.as_str().repeat(2);
                            target.replace_range(m.start()..m.end(), &r);
                        }
                    }
                    if cap.name("fill").is_some() {
                        dup_indexes.push(i);
                        target.replace_range(m.start()..m.end(), &spaces[0..(m.end() - m.start())]);
                    }
                }
                let count = target.chars().count();
                if count < maxl {
                    target.extend(std::iter::repeat_n(' ', maxl - count));
                }
                target
            })
            .collect();

        let h = if h > lines.len() { h } else { lines.len() };
        // Duplicate lines until we reach target height
        dup_lines(&dup_indexes, &mut lines, h);

        for (i, line) in lines.iter_mut().enumerate() {
            let mut clears = Vec::new();
            for cap in self.re.captures_iter(line) {
                let m = cap.get(0).unwrap();
                if let Some(x) = cap.name("var") {
                    let n: String;
                    if let Some(new_name) = self.renames.get(x.as_str()) {
                        n = new_name.clone();
                    } else {
                        n = x.as_str().to_string();
                    }
                    let mut color: u32 = 0xff_ff_ff;
                    if let Some(new_color) = self.colors.get(x.as_str()) {
                        color = *new_color;
                    }

                    let col = line[..m.start()].chars().count();
                    let len = line[m.start()..m.end()].chars().count();
                    data.insert(
                        n,
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
                line.replace_range(start..end, &spaces[0..(end - start)]);
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
