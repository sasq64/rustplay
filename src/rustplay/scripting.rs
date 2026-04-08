use anyhow::Result;
use crokey::KeyCombinationFormat;
use crossterm::event::KeyCode;
use crossterm::event::KeyEvent;
use crossterm::event::KeyEventState;
use crossterm::event::KeyModifiers;
use mlua::UserData;
use mlua::UserDataMethods;
use mlua::prelude::*;
use std::collections::HashMap;

/// Script override for a variable in the template string
#[derive(Default)]
pub struct TemplateVar {
    color: Option<u32>,
    alias: Option<String>,
    func: Option<LuaRegistryKey>,
}

/// Result of overriding
#[derive(Clone, Debug, Default)]
pub struct Override {
    pub color: Option<u32>,
    pub alias: Option<String>,
    pub value: Value,
}

use crate::Settings;
use crate::rustplay::song::FileInfo;
use crate::rustplay::state::InputMode;
use crate::{RustPlay, log, value::Value};

impl UserData for FileInfo {
    fn add_methods<M: UserDataMethods<Self>>(_methods: &mut M) {}
}

impl UserData for RustPlay {
    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        methods.add_method_mut("next_song", |_, this: &mut RustPlay, ()| {
            this.next_song();
            Ok(())
        });
        methods.add_method_mut("prev_song", |_, this: &mut RustPlay, ()| {
            this.prev_song();
            Ok(())
        });
        methods.add_method_mut("next_subtune", |_, this: &mut RustPlay, ()| {
            this.next_subtune();
            Ok(())
        });
        methods.add_method_mut("prev_subtune", |_, this: &mut RustPlay, ()| {
            this.prev_subtune();
            Ok(())
        });
        methods.add_method_mut("set_song", |_, this: &mut RustPlay, (song,): (u32,)| {
            this.set_song(song);
            Ok(())
        });
        methods.add_method_mut("goto_parent", |_, this: &mut RustPlay, ()| {
            this.goto_parent().map_err(mlua::Error::external)
        });
        methods.add_method_mut("focus_search_edit", |_, this: &mut RustPlay, ()| {
            this.focus_search_edit();
            Ok(())
        });
        methods.add_method_mut("show_main", |_, this: &mut RustPlay, ()| {
            this.show_main();
            Ok(())
        });
        methods.add_method_mut("show_search_result", |_, this: &mut RustPlay, ()| {
            this.show_search_result();
            Ok(())
        });
        methods.add_method_mut("show_directory", |_, this: &mut RustPlay, ()| {
            this.show_directory().map_err(mlua::Error::external)
        });
        methods.add_method_mut("show_favorites", |_, this: &mut RustPlay, ()| {
            this.show_favorites();
            Ok(())
        });
        methods.add_method_mut("show_current", |_, this: &mut RustPlay, ()| {
            this.show_current().map_err(mlua::Error::external)
        });
        methods.add_method_mut("enter_or_play_selected", |_, this: &mut RustPlay, ()| {
            this.enter_or_play_selected().map_err(mlua::Error::external)
        });
        methods.add_method_mut("play_pause", |_, this: &mut RustPlay, ()| {
            this.play_pause();
            Ok(())
        });
        methods.add_method_mut(
            "add_favorite",
            |_, this: &mut RustPlay, (song,): (LuaUserDataRef<FileInfo>,)| {
                log!("Add fav");
                let file_info: FileInfo = song.clone();
                this.add_favorite(file_info.clone());
                Ok(())
            },
        );
        methods.add_method_mut("quit", |_, this: &mut RustPlay, ()| {
            this.quit();
            Ok(())
        });
        methods.add_method("get_selected_song", |_, this: &RustPlay, ()| {
            Ok(this.get_selected_song())
        });
        methods.add_method("get_playing_song", |_, this: &RustPlay, ()| {
            Ok(this.get_playing_song())
        });
        methods.add_method("input_mode", |_, this: &RustPlay, ()| {
            Ok(match this.input_mode() {
                InputMode::Main => "n",
                InputMode::SearchInput => "s",
                InputMode::DirScreen => "d",
                InputMode::SearchScreen => "s",
                InputMode::FavScreen => "f",
                InputMode::ResultScreen => "r",
            })
        });
        methods.add_method_mut("add_char", |_, this: &mut RustPlay, (s,): (String,)| {
            let ke: KeyEvent = crokey::parse(&s).map_err(mlua::Error::external)?.into();
            this.add_char(ke).map_err(mlua::Error::external)
        });
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Hash)]
enum MappedKey {
    Code(KeyCode, KeyModifiers),
    Digit,
    Letter,
}

pub(crate) struct Scripting {
    lua: Lua,
    template: String,
    variables: HashMap<String, TemplateVar>,
    keys: HashMap<InputMode, HashMap<MappedKey, LuaFunction>>,
    pub info: Option<String>,
    pub settings: Option<Settings>,
}

impl Scripting {
    pub fn get_template(&self) -> String {
        self.template.clone()
    }

    pub fn new(script: impl Into<String>) -> Result<Self> {
        let lua = Lua::new();

        lua.globals().set(
            "log",
            lua.create_function(|_, t: String| {
                log!("LUA: {t}");
                Ok(())
            })?,
        )?;

        let prelude = r#"
function play_pause() rust_play:play_pause() end
function next_song() rust_play:next_song() end
function prev_song() rust_play:prev_song() end
function next_subtune() rust_play:next_subtune() end
function prev_subtune() rust_play:prev_subtune() end
function sub_song(n) rust_play:set_song(n) end
function goto_parent() rust_play:goto_parent() end
function show_favorites() rust_play:show_favorites() end
function show_directory() rust_play:show_directory() end
function show_main() rust_play:show_main() end
function focus_search() rust_play:focus_search_edit() end
function quit() rust_play:quit() end
function get_selected_song() return rust_play:get_selected_song() end
function get_playing_song() return rust_play:get_playing_song() end
function add_favorite(song) rust_play:add_favorite(song) end
function add_char(c) rust_play:add_char(c) end
function show_current() rust_play:show_current() end
function enter_or_play_selected() rust_play:enter_or_play_selected() end
"#;
        lua.load(prelude).exec()?;

        let mut settings: Option<Settings> = None;

        let mut template = String::new();
        let mut variables = HashMap::<String, TemplateVar>::new();
        let mut keys = HashMap::<InputMode, HashMap<MappedKey, LuaFunction>>::new();

        let modes: HashMap<_, _> = [
            ('n', InputMode::Main),
            ('f', InputMode::FavScreen),
            ('d', InputMode::DirScreen),
            ('s', InputMode::SearchScreen),
            ('i', InputMode::SearchInput),
        ]
        .into();

        for mode in modes.values() {
            keys.insert(*mode, HashMap::new());
        }
        let mut info = None;

        let table: mlua::Table = lua.load(script.into()).eval()?;
        for pair in table.pairs::<mlua::Value, mlua::Value>() {
            let (key, value) = pair?;
            let key_str = match key {
                mlua::Value::String(s) => s.to_str()?.to_string(),
                _ => continue,
            };
            match key_str.as_str() {
                "settings" => {
                    settings = Some(lua.from_value::<Settings>(value)?);
                }
                "info" => {
                    info = Some(value.to_string()?.clone());
                }
                "template" => {
                    let val = value.to_string()?;
                    template = val.clone();
                    //shared_state.borrow_mut().template = val;
                }
                "vars" => {
                    let t = value.as_table().unwrap();
                    for pair in t.pairs::<String, LuaTable>() {
                        let (key, map) = pair?;
                        let mut tvar = TemplateVar::default();
                        if let Ok(alias) = map.get::<String>("alias_for") {
                            tvar.alias = Some(alias);
                        }
                        if let Ok(color) = map.get::<i64>("color") {
                            tvar.color = Some(color as u32);
                        }
                        if let Ok(func) = map.get::<LuaFunction>("func") {
                            tvar.func = Some(lua.create_registry_value(func)?);
                        }
                        variables.insert(key, tvar);
                    }
                }
                "keys" => {
                    let t = value.as_table().unwrap();
                    for item in t.sequence_values::<LuaTable>().flatten() {
                        let mut mode = item.get::<String>(1)?;
                        mode = mode.replace("a", "nidfs");
                        mode = mode.replace("r", "dfs");
                        let key = item.get::<String>(2)?;
                        for key in key.split(',') {
                            log!("KEY {key} MODE {mode}");
                            let mk = if key == ":digit:" {
                                MappedKey::Digit
                            } else if key == ":letter:" {
                                MappedKey::Letter
                            } else {
                                let ke: KeyEvent = crokey::parse(key)?.into();
                                MappedKey::Code(ke.code, ke.modifiers)
                            };

                            let action = item.get::<mlua::Value>(3)?;
                            for c in mode.chars() {
                                let Some(input_mode) = modes.get(&c) else {
                                    anyhow::bail!("config.lua: Key '{key}' has illegal mode '{c}'");
                                };
                                match &action {
                                    mlua::Value::Function(f) => {
                                        keys.get_mut(input_mode)
                                            .expect("All mode maps should exist")
                                            .insert(mk.clone(), f.clone());
                                    }
                                    _ => {
                                        anyhow::bail!(
                                            "config.lua: Key action '{key}' maps to {:?}",
                                            action
                                        );
                                    }
                                }
                            }
                        }
                    }
                }

                _ => {}
            }
        }

        Ok(Scripting {
            lua,
            template,
            variables,
            keys,
            info,
            settings,
        })
    }

    pub fn handle_key(
        &mut self,
        rust_play: &mut RustPlay,
        code: KeyCode,
        modifiers: KeyModifiers,
        mode: InputMode,
    ) -> Result<bool> {
        let fmt = KeyCombinationFormat::default();
        let ke = KeyEvent {
            code,
            modifiers,
            kind: crossterm::event::KeyEventKind::Press,
            state: KeyEventState::NONE,
        };
        let text = fmt.to_string(ke);
        log!("TEXT: {text}");

        // if let KeyCode::Char(c) = code {
        //     text = c.to_string();
        // }
        let mut mapped_keys = vec![MappedKey::Code(code, modifiers)];
        if let KeyCode::Char(d) = code
            && modifiers == KeyModifiers::NONE
        {
            if d.is_ascii_digit() {
                mapped_keys.push(MappedKey::Digit);
            }
            if d.is_alphabetic() {
                mapped_keys.push(MappedKey::Letter);
            }
        };

        let keys = self.keys.get(&mode).expect("All mode maps should exists");
        for mapped_key in mapped_keys.into_iter() {
            if let Some(f) = keys.get(&mapped_key) {
                let _ = self.lua.scope(|scope| {
                    let ud = scope.create_userdata_ref_mut(rust_play)?;
                    self.lua.globals().set("rust_play", ud)?;
                    f.call::<bool>((text.clone(),))
                })?;
                return Ok(true);
            }
        }
        Ok(false)
    }

    pub fn get_settings(&self) -> Settings {
        self.settings.clone().unwrap_or_default()
    }

    /// Ask the script for custom colors and values for metadata placeholders
    pub fn get_overrides(
        &self,
        meta: &HashMap<String, Value>,
    ) -> Result<HashMap<String, Override>> {
        let mut result = HashMap::new();

        let lua_meta = self.lua.create_table()?;
        for (k, v) in meta {
            match v {
                Value::Text(s) => lua_meta.set(k.as_str(), s.as_str())?,
                Value::Number(n) => lua_meta.set(k.as_str(), *n)?,
                _ => lua_meta.set(k.as_str(), "")?,
            }
        }

        for (name, tvar) in &self.variables {
            let value = match &tvar.func {
                Some(key) => {
                    let func: LuaFunction = self.lua.registry_value(key)?;
                    let s: String = func.call(lua_meta.clone())?;
                    Value::Text(s)
                }
                None => Value::Unknown,
            };
            result.insert(
                name.clone(),
                Override {
                    color: tvar.color,
                    alias: tvar.alias.clone(),
                    value,
                },
            );
        }
        Ok(result)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use std::collections::HashMap;

    use crate::{rustplay::scripting::Scripting, value::Value};

    #[test]
    fn test_scripting() {
        let script = r#"

local vars = {
  a = { alias = "one" },
  b = { color = 0xff8040 },
}
return {
    vars = vars,
    template = "hello",
    keys = {},
    settings = {}
}
        "#;

        let scripting = Scripting::new(script).unwrap();
        let templ = scripting.get_template();
        assert_eq!(templ, "hello");
        let meta = HashMap::from([("c".to_string(), Value::Text("hey".into()))]);
        let vars = scripting.get_overrides(&meta).unwrap();
        println!("{vars:?}");

        let o = vars.get("b").unwrap();
        assert_eq!(o.color, Some(0xff8040));
    }
}
