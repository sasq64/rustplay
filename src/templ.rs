use crossterm::style::{Color, SetForegroundColor};
use regex::Regex;
use std::borrow::Borrow;
use std::collections::HashMap;
use std::fmt::Display;
use std::hash::Hash;

use std::io::{self, stdout};

use crossterm::{cursor, style::Print, QueueableCommand};

struct PlaceHolder {
    start: usize,
    end: usize,
    col: usize,
    line: usize,
    color: u32,
}

pub struct Template {
    templ: Vec<String>,
    data: HashMap<String, PlaceHolder>,
}

impl Template {
    fn as_string(&self) -> String {
        self.templ.join("\n")
    }

    pub fn render<T: Display, Q: Hash + Eq + Borrow<str>>(
        &self,
        data: &HashMap<Q, T>,
    ) -> Vec<String> {
        let mut result = self.templ.clone();
        for (key, val) in data {
            if let Some(ph) = self.data.get(key.borrow()) {
                let line = &mut result[ph.line];
                let text = format!("{}", val);
                let mut end = ph.start + text.len();
                if end > line.len() {
                    end = line.len();
                }

                line.replace_range(ph.start..end, &text);
            }
        }
        result
    }

    pub fn write<T: Display, Q: Hash + Eq + Borrow<str>>(
        &self,
        data: &HashMap<Q, T>,
        x: u16,
        y: u16,
    ) -> io::Result<()> {
        for (i, line) in self.templ.iter().enumerate() {
            let _ = stdout()
                .queue(cursor::MoveTo(x, y + i as u16))?
                .queue(Print(line))?;
        }

        for (key, val) in data {
            if let Some(ph) = self.data.get(key.borrow()) {
                let text = format!("{}", val);
                let r = (ph.color >> 16) as u8;
                let g = ((ph.color >> 8) & 0xff) as u8;
                let b = (ph.color & 0xff) as u8;
                let _ = stdout()
                    .queue(cursor::MoveTo(x + ph.col as u16, y + ph.line as u16))?
                    .queue(SetForegroundColor(Color::Rgb { r, g, b }))?
                    .queue(Print(text))?;
            }
        }
        Ok(())
    }

    pub fn get_pos(&self, id: &str) -> Option<(u16, u16)> {
        if let Some(ph) = self.data.get(id) {
            return Some((ph.col as u16, ph.line as u16));
        }
        None
    }

    pub fn render_string<T: Display, Q: Hash + Eq + Borrow<str>>(
        &self,
        data: &HashMap<Q, T>,
    ) -> String {
        let result = self.render(data);
        result.join("\n")
    }

    /// Template contains text or special patterns that are replaced
    /// `$>` Pattern removed, previous character is repeated until current line length = target length
    /// `$^` Pattern replaced with spaces, current line becomes part of vertical resize and may be
    ///      duplicated any number of times
    ///  `$<symbol>` Pattern first replaced with spaces then with value from hashmap
    ///
    /// Special lines
    ///
    /// Variable alias
    /// @short_symbol = real_symbol
    ///
    pub fn new(templ: &str, w: usize, h: usize) -> Template {
        let spaces = "                                                                   ";
        let re = Regex::new(r"\$((\w+)|>(.)|(\^))").unwrap();
        let re2 = Regex::new(r"\@(\w+)=(\w+)?(:#([a-fA-F0-9]+))?").unwrap();
        let mut out = Vec::<String>::new();
        let mut data = HashMap::<String, PlaceHolder>::new();
        let mut dup_lines = Vec::new();

        let mut renames: HashMap<&str, &str> = HashMap::new();
        let mut colors: HashMap<&str, u32> = HashMap::new();

        let mut lines: Vec<&str> = templ.lines().collect();
        lines.retain(|line| {
            if let Some(m) = re2.captures(line) {
                println!("@MATCH {}", m.get(0).unwrap().as_str());
                let var = m.get(1).unwrap().as_str();
                if let Some(alias) = m.get(2) {
                    renames.insert(alias.as_str(), var);
                }
                if let Some(color) = m.get(4) {
                    println!("@COLOR {}", color.as_str());
                    let rgb = u32::from_str_radix(color.as_str(), 16).unwrap();
                    println!("@COLOR {}", rgb);
                    colors.insert(var, rgb);
                }
                return false;
            }
            true
        });

        println!("PASS 1");
        for (i, line) in lines.iter_mut().enumerate() {
            //let mut target: Vec<char> = line.chars().collect();
            let mut target = line.to_owned();
            for cap in re.captures_iter(line) {
                let m = cap.get(0).unwrap();
                println!("MATCH '{}'", m.as_str());
                if let Some(x) = cap.get(3) {
                    let target_len = target.chars().count();
                    if w > target_len {
                        println!("W {} T {}", w, target_len);
                        let len = (w - target_len) + 3;
                        println!("LINE FILL {} LEN {}", x.as_str(), len);
                        let r = x.as_str().repeat(len);
                        target.replace_range(m.start()..m.end(), &r);
                    } else {
                        let r = x.as_str().repeat(2);
                        target.replace_range(m.start()..m.end(), &r);
                    }
                }
                if cap.get(4).is_some() {
                    dup_lines.push(i);
                    target.replace_range(m.start()..m.end(), &spaces[0..(m.end() - m.start())]);
                }
            }
            out.push(target);
        }

        let s = (h - out.len()) as f32 / dup_lines.len() as f32;
        let mut n = 0;
        let mut f = 0.0;
        for i in dup_lines.iter().rev() {
            f += s;
            while (n as f32) < f {
                out.insert(*i, out[*i].clone());
                n += 1;
            }
        }
        println!("PASS 2");
        for (i, target) in out.iter_mut().enumerate() {
            //let mut target: Vec<char> = line.chars().collect();
            let mut clears = Vec::new();
            for cap in re.captures_iter(target) {
                let m = cap.get(0).unwrap();
                println!("MATCH {}", m.as_str());
                if let Some(x) = cap.get(2) {
                    // Word
                    println!("WORD {}", x.as_str());
                    let n: &str;
                    if let Some(new_name) = renames.get(x.as_str()) {
                        n = new_name;
                    } else {
                        n = x.as_str();
                    }
                    let mut color: u32 = 0xffffff;
                    if let Some(new_color) = colors.get(x.as_str()) {
                        color = *new_color;
                    }
                    let col = target[..m.start()].chars().count();
                    data.insert(
                        n.to_owned(),
                        PlaceHolder {
                            start: m.start(),
                            end: m.end(),
                            col,
                            line: i,
                            color,
                        },
                    );
                    clears.push((m.start(), m.end()));
                }
            }
            for (start, end) in clears {
                target.replace_range(start..end, &spaces[0..(end - start)]);
            }
        }
        Template { templ: out, data }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use crate::player::Value;

    use super::Template;

    #[test]
    fn template_works() {
        let data = HashMap::from([("one", 9)]);
        let result = Template::new("Line $one\nX $x!\n---$>--", 10, 3);
        let text = result.as_string();
        println!(":: {}", text);
        assert!(text == "Line     \nX   !\n----------");

        assert!(result.data["one"].start == 5);

        let r = result.render_string(&data);
        println!("{}", r);

        let result = Template::new(
            r#"@pooh=asda
+----+$>---------+
|$^  | $hello $> |
+----+--------$>-+
|$^  |$>         |
+--=-+--$>------+
@name=gargamel
"#,
            40,
            18,
        );

        let text = result.render_string(&HashMap::from([("hello", "DOG!")]));
        println!("{}", text);
        assert!(text == "Line     \nX   !\n----------");
    }

    #[test]
    fn player_templ_works() {
        let mut song_meta = HashMap::<String, Value>::new();
        song_meta.insert(
            "full_title".to_string(),
            Value::Text("Enigma (Musiklinjen)".to_string()),
        );
        song_meta.insert("isong".to_string(), Value::Number(2));
        song_meta.insert("len".to_string(), Value::Number(100));
        song_meta.insert("xxx".to_string(), Value::Data(Vec::<u8>::new()));

        //let templ = Template::new("TITLE:    $full_title\n          $sub_title\nCOMPOSER: $composer\nFORMAT:   $format\n\nTIME: 00:00:00 ($len) SONG: $isong/$songs", 60, 10);
        let templ = Template::new(include_str!("../screen.templ"), 80, 10);
        let x = templ.render_string(&song_meta);
        println!("{}", x);
        panic!();
    }
}
