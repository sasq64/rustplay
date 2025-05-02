use anyhow::Result;
use crossterm::style::{Color, SetForegroundColor};
use regex::Regex;
use std::borrow::Borrow;
use std::collections::HashMap;
use std::fmt::Display;
use std::hash::Hash;

use std::io::{self, stdout};

use crossterm::{QueueableCommand, cursor, style::Print};

struct PlaceHolder {
    start: usize,
    end: usize,
    col: usize,
    len: usize,
    line: usize,
    color: u32,
}

pub struct Template {
    templ: Vec<String>,
    use_color: bool,
    data: HashMap<String, PlaceHolder>,
}

fn color(color: u32) -> Color {
    let r = (color >> 16) as u8;
    let g = ((color >> 8) & 0xff) as u8;
    let b = (color & 0xff) as u8;
    Color::Rgb { r, g, b }
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
    pub fn write<T: Display, Q: Hash + Eq + Borrow<str>>(
        &self,
        data: &HashMap<Q, T>,
        x: u16,
        y: u16,
    ) -> io::Result<()> {
        for (i, line) in self.templ.iter().enumerate() {
            stdout()
                .queue(cursor::MoveTo(x, y + i as u16))?
                .queue(Print(line))?;
        }

        for (key, val) in data {
            self.write_field(x, y, key.borrow(), val)?;
        }
        Ok(())
    }

    pub fn write_field<T: Display>(
        &self,
        x: u16,
        y: u16,
        key: &str,
        val: &T,
    ) -> io::Result<()> {
        if let Some(ph) = self.data.get(key) {
            let text = format!("{val}");
            let l = usize::min(text.len(), ph.len);
            if self.use_color {
                stdout().queue(SetForegroundColor(color(ph.color)))?;
            }
            stdout()
                .queue(cursor::MoveTo(x + ph.col as u16, y + ph.line as u16))?
                .queue(Print(&text[..l]))?;
        }
        Ok(())
    }

    pub fn get_pos(&self, id: &str) -> Option<(u16, u16)> {
        if let Some(ph) = self.data.get(id) {
            return Some((ph.col as u16, ph.line as u16));
        }
        None
    }

    // For testing?

    fn render<T: Display, Q: Hash + Eq + Borrow<str>>(
        &self,
        data: &HashMap<Q, T>,
    ) -> Vec<String> {
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

    pub fn set_use_color(&mut self, c: bool) {
        self.use_color = c;
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
        let spaces = "                                                                   ";
        let alias_re = Regex::new(r"\@(\w+)=(\w+)?(:#([a-fA-F0-9]+))?")?;

        let mut data = HashMap::<String, PlaceHolder>::new();
        let mut dup_indexes = Vec::new();

        let mut renames: HashMap<&str, &str> = HashMap::new();
        let mut colors: HashMap<&str, u32> = HashMap::new();

        let maxl = w;

        // Strip alias assignments from template
        let lines: Vec<&str> = templ
            .lines()
            .filter(|line| {
                if let Some(m) = alias_re.captures(line) {
                    if let Some(var) = m.get(1) {
                        if let Some(alias) = m.get(2) {
                            renames.insert(alias.as_str(), var.as_str());
                        }
                        if let Some(color) = m.get(4) {
                            if let Ok(rgb) = u32::from_str_radix(color.as_str(), 16) {
                                colors.insert(var.as_str(), rgb);
                            }
                        }
                    }
                    return false;
                }
                true
            })
            .collect();
        // Find fill patterns ($> and $^), resize vertically and prepare for horizontal
        let re = Regex::new(r"\$(((?<var>\w+)\s*)|>(?<char>.)|(?<fill>\^))")?;
        // Captures: var = var_name, char = char to repeat for '$>',
        // fill = '^' for line dup
        let mut lines: Vec<String> = lines
            .iter()
            .enumerate()
            .map(|(i, &line)| {
                //let mut target: Vec<char> = line.chars().collect();
                let mut target = line.to_string();
                for cap in re.captures_iter(line) {
                    let m = cap.get(0).unwrap();
                    //println!("MATCH '{}'", m.as_str());
                    if let Some(x) = cap.name("char") {
                        let target_len = target.chars().count();
                        if w > target_len {
                            //println!("W {} T {}", w, target_len);
                            let len = (w - target_len) + 3;
                            //println!("LINE FILL {} LEN {}", x.as_str(), len);
                            let r = x.as_str().repeat(len);
                            target.replace_range(m.start()..m.end(), &r);
                        } else {
                            let r = x.as_str().repeat(2);
                            target.replace_range(m.start()..m.end(), &r);
                        }
                    }
                    if cap.name("fill").is_some() {
                        dup_indexes.push(i);
                        target.replace_range(
                            m.start()..m.end(),
                            &spaces[0..(m.end() - m.start())],
                        );
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
            //let mut target: Vec<char> = line.chars().collect();
            let mut clears = Vec::new();
            for cap in re.captures_iter(line) {
                let m = cap.get(0).unwrap();
                //println!("MATCH {}", m.as_str());
                if let Some(x) = cap.name("var") {
                    let n: &str;
                    if let Some(new_name) = renames.get(x.as_str()) {
                        n = new_name;
                    } else {
                        n = x.as_str();
                    }
                    let mut color: u32 = 0xff_ff_ff;
                    if let Some(new_color) = colors.get(x.as_str()) {
                        color = *new_color;
                    }

                    let col = line[..m.start()].chars().count();
                    let len = line[m.start()..m.end()].chars().count();
                    data.insert(
                        n.to_owned(),
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
        Ok(Template {
            templ: lines,
            use_color: false,
            data,
        })
    }

    #[allow(clippy::unused_self)]  
    pub fn set_vars(&mut self, _variables: HashMap<String, crate::TemplateVar>) {
        //self.variables = variables;
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
