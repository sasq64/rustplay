use anyhow::Result;
use crossterm::style::Color;
use std::io::Cursor;
use std::path::Path;
use std::{fs, io};

pub fn extract_zip(data_zip: &[u8], dd: &Path) -> Result<()> {
    let cursor = Cursor::new(data_zip);
    let mut archive = zip::ZipArchive::new(cursor)?;
    for i in 0..archive.len() {
        let mut file = archive.by_index(i)?;
        let outpath = match file.enclosed_name() {
            Some(path) => dd.join(path),
            None => continue,
        };
        if file.is_dir() {
            fs::create_dir_all(&outpath)?;
        } else {
            if let Some(p) = outpath.parent()
                && !p.exists()
            {
                fs::create_dir_all(p)?;
            }
            let mut outfile = fs::File::create(&outpath)?;
            io::copy(&mut file, &mut outfile)?;
        }
    }
    Ok(())
}

pub fn make_color(color: u32) -> Color {
    let r = (color >> 16) as u8;
    let g = ((color >> 8) & 0xff) as u8;
    let b = (color & 0xff) as u8;
    Color::Rgb { r, g, b }
}
