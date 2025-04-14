#![allow(dead_code)]

use std::collections::{HashMap, VecDeque};
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::Receiver;
use std::sync::{Arc, Mutex, mpsc};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use musix::SongInfo;
use tantivy::collector::TopDocs;
use tantivy::query::QueryParser;

use anyhow::Context;
use anyhow::Result;
use anyhow::anyhow;
use tantivy::{Index, IndexWriter, ReloadPolicy, doc};
use tantivy::{IndexReader, schema::*};
use walkdir::WalkDir;

use crate::value;

const FORMAT_DIRS: [&str; 327] = [
    "Actionamics",
    "Activision Pro",
    //"Ad Lib",
    "Aero Studio",
    "AHX",
    "All Sound Tracker",
    "AM Composer",
    "Anders Oland",
    "AND XSynth",
    "AProSys",
    "ArkosTracker",
    "Art And Magic",
    "Art Of Noise",
    "Asylum",
    "Atari Digi-Mix",
    "Athtune",
    "Audio Sculpture",
    "AXS",
    "AY Amadeus",
    "AY Emul",
    "AY STRC",
    "Beathoven Synthesizer",
    "Beaver Sweeper",
    "Beepola",
    "Ben Daglish",
    "Ben Daglish SID",
    "BeRoTracker",
    "BoyScout",
    "BP SoundMon 2",
    "BP SoundMon 3",
    "Buzz",
    "Buzzic 1.0",
    "Buzzic 1.1",
    "Buzzic 2.0",
    "Capcom Q-Sound Format",
    "CBA",
    "Cinemaware",
    "Composer 669",
    "Compoz",
    "Core Design",
    "Cubic Tiny XM",
    "CustomMade",
    "Cybertracker",
    "Cybertracker C64",
    "Darius Zendeh",
    "DarkWave Studio",
    "Dave Lowe",
    "Dave Lowe New",
    "David Hanney",
    "David Whittaker",
    "Delitracker Custom",
    "Delta Music",
    "Delta Music 2",
    "Delta Packer",
    "Desire",
    "Digibooster",
    "Digibooster Pro",
    "Digital-FM Music",
    "Digital Mugician",
    "Digital Mugician 2",
    "Digital Sonix And Chrome",
    "Digital Sound And Music Interface",
    "Digital Sound Interface Kit",
    "Digital Sound Interface Kit RIFF",
    "Digital Sound Studio",
    "DigitalTracker",
    "Digital Tracker DTM",
    "Digital Tracker MOD",
    "Digitrakker",
    "Digitrekker",
    "Dirk Bialluch",
    "Disorder Tracker 2",
    "Dreamcast Sound Format",
    "DreamStation",
    "Dynamic Studio Professional",
    "Dynamic Synthesizer",
    "EarAche",
    "Electronic Music System",
    "Electronic Music System v6",
    "Epic Megagames MASI",
    "Extreme Tracker",
    "Face The Music",
    "FamiTracker",
    "Farandole Composer",
    "Fashion Tracker",
    "Fasttracker",
    "Fasttracker 2",
    "Follin Player II",
    "Forgotten Worlds",
    "Fred Gray",
    "FredMon",
    "FuchsTracker",
    "Funktracker",
    "Future Composer 1.3",
    "Future Composer 1.4",
    "Future Composer BSI",
    "Future Player",
    "Gameboy Sound Format",
    "Gameboy Sound System",
    "Gameboy Sound System GBR",
    "Gameboy Tracker",
    "Game Music Creator",
    "General DigiMusic",
    "GlueMon",
    "GoatTracker",
    "GoatTracker 2",
    "GoatTracker Stereo",
    "Graoumf Tracker",
    "Graoumf Tracker 2",
    "GT Game Systems",
    "HES",
    "Hippel",
    "Hippel 7V",
    "Hippel-Atari",
    "Hippel-COSO",
    "Hippel-ST",
    "HivelyTracker",
    "Howie Davies",
    "Images Music System",
    "Imago Orpheus",
    "Impulsetracker",
    "InStereo!",
    "InStereo! 2.0",
    "Ixalance",
    "JamCracker",
    "Janko Mrsic-Flogel",
    "Jason Brooke",
    "Jason Page",
    "Jason Page Old",
    "JayTrax",
    "Jeroen Tel",
    "Jesper Olsen",
    "Ken's Digital Music",
    "Klystrack",
    "Kris Hatlelid",
    "KSS",
    "Leggless Music Editor",
    "Lionheart",
    "Liquid Tracker",
    "Mad Tracker 2",
    "Magnetic Fields Packer",
    "Maniacs Of Noise",
    "Maniacs Of Noise Old",
    "Mark Cooksey",
    "Mark Cooksey Old",
    "Mark II",
    "MaxTrax",
    "MCMD",
    //"MDX",
    "Medley",
    "Megadrive CYM",
    "Megadrive GYM",
    "MegaStation",
    "MegaStation MIDI",
    "Megatracker",
    "Mike Davies",
    "MikMod UNITRK",
    "Monotone",
    "MultiMedia Sound",
    "Multitracker",
    "Music Assembler",
    "Music Editor",
    "Musicline Editor",
    "MusicMaker",
    "MusicMaker v8",
    "MVS Tracker",
    "MVX Module",
    "NerdTracker 2",
    "Nintendo DS Sound Format",
    "Nintendo Sound Format",
    "Nintendo SPC",
    "NoiseTrekker",
    "NoiseTrekker 2",
    "NovoTrade Packer",
    "Octalyser",
    "OctaMED MMD0",
    "OctaMED MMD1",
    "OctaMED MMD2",
    "OctaMED MMD3",
    "OctaMED MMDC",
    "Oktalyzer",
    "Onyx Music File",
    "Organya",
    "Organya 2",
    "Paul Robotham",
    "Paul Shields",
    "Paul Summers",
    "Peter Verswyvelen",
    "Picatune",
    "Picatune2",
    "Pierre Adane Packer",
    "Piston Collage",
    "Piston Collage Protected",
    "PlayerPro",
    "PlaySID",
    "Playstation Sound Format",
    "PMD",
    "PokeyNoise",
    "Pollytracker",
    "Polytracker",
    "Powertracker",
    "Professional Sound Artists",
    "Protracker",
    "Protracker 3.6",
    "ProTrekkr",
    "ProTrekkr 2.0",
    "Psycle",
    "Pumatracker",
    "Quadra Composer",
    "Quartet PSG",
    "Quartet ST",
    "RamTracker",
    "RealSID",
    "Real Tracker",
    "Renoise",
    "Renoise 1.8",
    "Renoise 2.0",
    "Renoise 2.1",
    "Renoise 2.5",
    "Renoise 2.7",
    "Renoise Old",
    "Richard Joseph",
    "Riff Raff",
    "Rob Hubbard",
    "Rob Hubbard 2",
    "Rob Hubbard ST",
    "Ron Klaren",
    "S98",
    "Sam Coupe COP",
    "Sam Coupe SNG",
    "Saturn Sound Format",
    "SBStudio",
    "SC68",
    "Screamtracker 2",
    "Screamtracker 3",
    "SCUMM",
    "Sean Connolly",
    "Sean Conran",
    "Shroom",
    "SidMon 1",
    "SidMon 2",
    "Sidplayer",
    "Silmarils",
    "Skale Tracker",
    "Slight Atari Player",
    "SNDH",
    "Sonic Arranger",
    "Sound Club",
    "Sound Club 2",
    "SoundControl",
    "SoundFactory",
    "SoundFX",
    "SoundFX 2",
    "Sound Images",
    "Sound Master",
    "Sound Master II v1",
    "Sound Master II v3",
    "SoundPlayer",
    "Sound Programming Language",
    "SoundTracker 2.6",
    "SoundTracker Pro II",
    "Special FX",
    "Special FX ST",
    "Spectrum ASC Sound Master",
    "Spectrum Fast Tracker",
    "Spectrum Flash Tracker",
    "Spectrum Fuxoft AY Language",
    "Spectrum Global Tracker",
    "Spectrum Pro Sound Creator",
    "Spectrum Pro Sound Maker",
    "Spectrum Pro Tracker 1",
    "Spectrum Pro Tracker 2",
    "Spectrum Pro Tracker 3",
    "Spectrum Sound Tracker 1.1",
    "Spectrum Sound Tracker 1.3",
    "Spectrum Sound Tracker Pro",
    "Spectrum Sound Tracker Pro 2",
    "Spectrum SQ Tracker",
    "Spectrum ST Song Compiler",
    "Spectrum Vortex",
    "Spectrum Vortex Tracker II",
    "Spectrum ZXS",
    "Speedy A1 System",
    "Speedy System",
    "SPU",
    "Starkos",
    "Startrekker AM",
    "Stereo Sidplayer",
    "Steve Barrett",
    "Stonetracker",
    "SunTronic",
    "SunVox",
    "Super Nintendo Sound Format",
    "SVAr Tracker",
    "Symphonie",
    "Synder SNG-Player",
    "Synder SNG-Player Stereo",
    "Synder Tracker",
    "Synth Dream",
    "Synthesis",
    "Synth Pack",
    "SynTracker",
    "TCB Tracker",
    "TFM Music Maker",
    "TFMX",
    "TFMX ST",
    "The 0ok Amazing Synth Tracker",
    "The Holy Noise",
    "The Musical Enlightenment",
    "Thomas Hermann",
    "Tomy Tracker",
    "TSS",
    "Tunefish",
    "Ultra64 Sound Format",
    "Ultratracker",
    "Unique Development",
    "Unis 669",
    "V2",
    "Velvet Studio",
    "VGM Music Maker",
    "Vic-Tracker",
    "Video Game Music",
    "Voodoo Supreme Synthesizer",
    "Wally Beben",
    "WonderSwan",
    "X-Tracker",
    "YM",
    "YMST",
    "Zoundmonitor",
];
#[inline]
/// Convert 8 bit unicode to utf8 String
fn slice_to_string(slice: &[u8]) -> String {
    slice
        .iter()
        .take_while(|&&b| b != 0)
        .map(|&b| b as char)
        .collect()
}

#[derive(Debug, Clone, PartialEq)]
pub struct FileInfo {
    path: PathBuf,
    pub meta_data: HashMap<String, value::Value>,
}

impl FileInfo {
    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn get(&self, what: &str) -> &value::Value {
        self.meta_data.get(what).unwrap_or(&value::Value::Unknown())
    }

    pub fn title_and_composer(&self) -> String {
        let title = self.get("title");
        let composer = self.get("composer");
        format!("{title} / {composer}")
    }

    pub fn full_song_name(&self) -> String {
        let title = self.get("title");
        let composer = self.get("composer");
        let file_name = self.path.file_name().unwrap().to_str().unwrap();
        format!("{title} / {composer} ({file_name})")
    }

    pub fn title(&self) -> Option<String> {
        if let Some(value::Value::Text(title)) = self.meta_data.get("title") {
            return Some(title.to_owned());
        }
        None
    }
}

pub trait PlayList {
    fn next() -> Option<FileInfo>;
}

pub struct Indexer {
    schema: Schema,
    index: Index,
    index_writer: IndexWriter,
    reader: IndexReader,
    pub result: Vec<FileInfo>,

    title_field: Field,
    composer_field: Field,
    path_field: Field,
    file_field: Field,

    song_list: VecDeque<FileInfo>,
    count: AtomicUsize,
}

fn get_string(doc: &TantivyDocument, field: &Field) -> Result<String> {
    if let Some(path_val) = doc.get_first(*field) {
        return match path_val {
            OwnedValue::Str(name) => Ok(name.to_owned()),
            _ => Err(anyhow!("")),
        };
    }
    Ok("???".to_owned())
}

impl Indexer {
    pub fn new() -> Result<Self> {
        let mut schema_builder = Schema::builder();
        let title_field = schema_builder.add_text_field("title", TEXT | STORED);
        let composer_field = schema_builder.add_text_field("composer", TEXT | STORED);
        let path_field = schema_builder.add_text_field("path", STORED);
        let file_field = schema_builder.add_text_field("file_name", TEXT);
        let schema = schema_builder.build();

        let index = Index::create_in_ram(schema.clone());

        let index_writer: IndexWriter = index.writer(20_000_000)?;
        let reader = index
            .reader_builder()
            .reload_policy(ReloadPolicy::OnCommitWithDelay)
            .try_into()?;
        Ok(Self {
            schema,
            index,
            index_writer,
            reader,
            result: Vec::new(),
            title_field,
            composer_field,
            path_field,
            file_field,
            song_list: VecDeque::new(),
            count: 0.into(),
        })
    }

    pub fn add_path(&mut self, song_path: &Path) -> Result<()> {
        let segments = song_path
            .ancestors()
            .filter_map(|a| a.file_name())
            .filter_map(|a| a.to_str())
            .collect::<Vec<_>>();
        let l = segments.len();
        if l >= 3 && FORMAT_DIRS.contains(&segments[2]) {
            let info = SongInfo {
                title: song_path.file_stem().unwrap().to_str().unwrap().to_owned(),
                composer: segments[1].to_owned(),
                ..SongInfo::default()
            };
            return self.add_with_info(song_path, &info);
        } else if l >= 4 && FORMAT_DIRS.contains(&segments[3]) {
            let info = SongInfo {
                title: song_path.file_stem().unwrap().to_str().unwrap().to_owned(),
                game: segments[1].to_owned(),
                composer: segments[2].to_owned(),
                ..SongInfo::default()
            };
            return self.add_with_info(song_path, &info);
        }
        self.count.fetch_add(1, Ordering::Relaxed);
        self.index_writer.add_document(doc!(
                self.file_field => song_path.file_name().context("No filename")?.to_str().unwrap(),
                self.path_field => song_path.to_str().unwrap().to_owned()))?;
        if self.song_list.len() < 100 {
            let file_info = FileInfo {
                path: song_path.into(),
                meta_data: HashMap::new(),
            };
            self.song_list.push_back(file_info);
        }
        Ok(())
    }

    pub fn identify_song(path: &Path) -> Result<Option<SongInfo>> {
        if let Some(ext) = path.extension() {
            if ext == "sid" {
                let mut buf: [u8; 0x60] = [0; 0x60];
                File::open(path)?.read_exact(&mut buf)?;
                let title = slice_to_string(&buf[0x16..0x36]);
                let composer = slice_to_string(&buf[0x36..0x56]);
                return Ok(Some(SongInfo {
                    title,
                    composer,
                    ..SongInfo::default()
                }));
            }
        }
        Ok(musix::identify_song(path))
    }

    pub fn add_with_info(&mut self, song_path: &Path, info: &SongInfo) -> Result<()> {
        self.count.fetch_add(1, Ordering::Relaxed);
        self.index_writer.add_document(doc!(
                self.title_field => info.title.clone(),
                self.composer_field => info.composer.clone(),
                self.file_field => song_path.file_name().context("No filename")?
                                    .to_str().unwrap(),
                self.path_field => song_path.to_str().context("Illegal path")?
                                    .to_owned()))?;
        if self.song_list.len() < 100 {
            let file_info = FileInfo {
                path: song_path.into(),
                meta_data: HashMap::from([
                    ("title".into(), value::Value::Text(info.title.clone())),
                    ("composer".into(), value::Value::Text(info.composer.clone())),
                ]),
            };
            self.song_list.push_back(file_info);
        }
        Ok(())
    }

    pub fn next(&mut self) -> Option<FileInfo> {
        if self.song_list.is_empty() {
            return None;
        }
        self.song_list.pop_front()
    }

    pub fn commit(&mut self) -> Result<()> {
        self.index_writer.commit()?;
        self.reader.reload()?;
        Ok(())
    }

    pub fn search(&mut self, query: &str) -> Result<()> {
        let searcher = self.reader.searcher();
        let query_parser = QueryParser::for_index(
            &self.index,
            vec![self.title_field, self.composer_field, self.file_field],
        );
        let query = query_parser.parse_query(query)?;
        let top_docs = searcher.search(&query, &TopDocs::with_limit(10000))?;

        self.result.clear();
        for (_score, doc_address) in top_docs {
            let doc: TantivyDocument = searcher.doc(doc_address)?;
            let path = get_string(&doc, &self.path_field)?;
            let title = get_string(&doc, &self.title_field)?;
            let composer = get_string(&doc, &self.composer_field)?;
            self.result.push(FileInfo {
                path: path.into(),
                meta_data: HashMap::from([
                    ("title".into(), value::Value::Text(title)),
                    ("composer".into(), value::Value::Text(composer)),
                ]),
            });
        }
        Ok(())
    }
}

pub struct RemoteIndexer {
    indexer: Arc<Mutex<Indexer>>,
    sender: mpsc::Sender<Cmd>,
    index_thread: Option<JoinHandle<()>>,
}

#[derive(Debug, Clone, PartialEq)]
enum Cmd {
    AddPath(PathBuf),
}

impl RemoteIndexer {
    fn run(indexer: Arc<Mutex<Indexer>>, rx: Receiver<Cmd>) -> Result<()> {
        loop {
            let cmd = rx.recv()?;
            match cmd {
                Cmd::AddPath(path) => {
                    let mut now = Instant::now();
                    for entry in WalkDir::new(path) {
                        let p = entry.unwrap();
                        if p.file_type().is_file() {
                            if let Some(info) = Indexer::identify_song(p.path())? {
                                let mut il = indexer.lock().unwrap();
                                il.add_with_info(p.path(), &info)?;
                            } else {
                                let mut il = indexer.lock().unwrap();
                                il.add_path(p.path())?;
                            }
                        }
                        if now.elapsed() > Duration::from_millis(1000) {
                            indexer.lock().unwrap().commit()?;
                            now += Duration::from_millis(1000);
                        }
                    }
                    indexer.lock().unwrap().commit()?;
                }
            }
        }
    }

    pub fn new() -> Result<RemoteIndexer> {
        let indexer = Arc::new(Mutex::new(Indexer::new()?));
        let (sender, rx) = mpsc::channel::<Cmd>();

        let index_thread = Some({
            let indexer = indexer.clone();
            thread::Builder::new()
                .name("index_thread".into())
                .spawn(move || {
                    RemoteIndexer::run(indexer, rx).expect("Fail");
                })?
        });
        Ok(RemoteIndexer {
            indexer,
            sender,
            index_thread,
        })
    }

    pub fn add_path(&self, path: &Path) {
        self.sender.send(Cmd::AddPath(path.to_owned())).unwrap();
    }

    pub fn next(&self) -> Option<FileInfo> {
        return self.indexer.lock().unwrap().next();
    }

    pub fn search(&mut self, query: &str) -> Result<()> {
        let mut indexer = self.indexer.lock().unwrap();
        indexer.search(query)?;
        Ok(())
    }

    pub fn index_count(&self) -> usize {
        let i = self.indexer.lock().unwrap();
        i.count.load(Ordering::Relaxed)
    }

    pub fn commit(&self) {}

    pub fn get_songs(&self, start: usize, stop: usize) -> Result<Vec<FileInfo>> {
        let indexer = self.indexer.lock().unwrap();
        let song_len = indexer.result.len();
        if song_len == 0 {
            return Ok(Vec::new());
        }
        if stop > song_len {
            return Ok(indexer.result[start..song_len].to_vec());
        }
        Ok(indexer.result[start..stop].to_vec())
    }

    pub(crate) fn get_song(&self, index: usize) -> Option<FileInfo> {
        let indexer = self.indexer.lock().unwrap();
        indexer.result.get(index).cloned()
    }

    pub fn song_len(&self) -> usize {
        let indexer = self.indexer.lock().unwrap();
        indexer.result.len()
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use walkdir::WalkDir;

    use crate::rustplay::indexer::RemoteIndexer;

    use super::Indexer;

    #[test]
    #[allow(clippy::unwrap_used)]
    fn identify_works() {
        let path: PathBuf = "../musicplayer/music/C64/Ark_Pandora.sid".into();
        let info = Indexer::identify_song(&path).unwrap().unwrap();
        assert_eq!(info.title, "Ark Pandora");
    }

    #[test]
    #[allow(clippy::unwrap_used)]
    fn normal_search_works() {
        let mut indexer = Indexer::new().unwrap();
        let path: PathBuf = "../musicplayer/music/C64/Ark_Pandora.sid".into();
        let info = musix::identify_song(&path).unwrap();
        indexer.add_with_info(&path, &info).unwrap();
        indexer.commit().unwrap();
        indexer.search("pandora").unwrap();
        assert!(indexer.result.len() == 1);

        let path: PathBuf =
            "/home/sasq/Music/MODLAND/Fasttracker 2/Purple Motion/sil forever.xm".into();
        indexer.add_path(&path).unwrap();
        indexer.commit().unwrap();
        indexer.search("purple motion").unwrap();
        assert_eq!(indexer.result.len(), 1);

        let path: PathBuf = "../musicplayer/music".into();
        for entry in WalkDir::new(path) {
            let e = entry.unwrap();
            if e.path().is_file() {
                if let Some(info) = musix::identify_song(e.path()) {
                    indexer.add_with_info(e.path(), &info).unwrap();
                } else {
                    indexer.add_path(e.path()).unwrap();
                }
            }
        }
        indexer.commit().unwrap();
        indexer.search("hubbard").unwrap();
        assert!(indexer.result.len() > 3);
        indexer.search("horace").unwrap();
        assert!(indexer.result.len() == 1);
        indexer.search("ninja").unwrap();
        assert!(indexer.result.len() >= 3);
        indexer.search("xywizoqp").unwrap();
        assert!(indexer.result.is_empty());
    }

    #[test]
    #[allow(clippy::unwrap_used)]
    fn threaded_search_works() {
        let mut indexer = RemoteIndexer::new().unwrap();
        let path: PathBuf = "../musicplayer/music".into();
        indexer.add_path(&path);
        std::thread::sleep(std::time::Duration::from_millis(500));
        indexer.search("horace").unwrap();
        assert!(indexer.song_len() == 1);
        indexer.search("ninja").unwrap();
        assert!(indexer.song_len() == 3);
    }
}
