use mlua::prelude::*;
use std::{cell::RefCell, collections::HashMap, error::Error, rc::Rc};

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
    pub value: Value,
}

#[derive(Default)]
pub(crate) struct SharedState {
    template: String,
    variables: HashMap<String, TemplateVar>,
}

use crate::{log, value::Value};

pub(crate) struct Script {
    lua: Lua,
    shared_state: Rc<RefCell<SharedState>>,
}

impl Script {
    pub fn get_template(&self) -> String {
        self.shared_state.borrow().template.clone()
    }

    pub fn new(script: impl Into<String>) -> Result<Self, Box<dyn Error>> {
        let shared_state = Rc::new(RefCell::new(SharedState::default()));

        let lua = Lua::new();

        lua.globals().set(
            "template",
            lua.create_function({
                let ss = shared_state.clone();
                move |_, t: String| {
                    ss.borrow_mut().template = t;
                    Ok(())
                }
            })?,
        )?;

        lua.globals().set(
            "log",
            lua.create_function(|_, t: String| {
                log!("LUA: {t}");
                Ok(())
            })?,
        )?;

        lua.globals().set(
            "set_meta",
            lua.create_function(|_, _: (String, String)| Ok(()))?,
        )?;

        lua.globals().set(
            "set_vars",
            lua.create_function({
                let ss = shared_state.clone();
                move |lua, vars: LuaTable| {
                    for pair in vars.pairs::<String, LuaTable>() {
                        let (key, map) = pair?;
                        let mut tvar = TemplateVar::default();
                        if let Ok(alias) = map.get::<String>("alias") {
                            tvar.alias = Some(alias);
                        }
                        if let Ok(color) = map.get::<i64>("color") {
                            tvar.color = Some(color as u32);
                        }
                        if let Ok(func) = map.get::<LuaFunction>("func") {
                            tvar.func = Some(lua.create_registry_value(func)?);
                        }
                        ss.borrow_mut().variables.insert(key, tvar);
                    }
                    Ok(())
                }
            })?,
        )?;

        lua.load(script.into()).exec()?;

        Ok(Script { lua, shared_state })
    }

    /// Ask the script for custom colors and values for metadata placeholders
    pub fn get_overrides(
        &self,
        meta: &HashMap<String, Value>,
    ) -> Result<HashMap<String, Override>, Box<dyn Error>> {
        let mut result = HashMap::new();

        let lua_meta = self.lua.create_table()?;
        for (k, v) in meta {
            match v {
                Value::Text(s) => lua_meta.set(k.as_str(), s.as_str())?,
                Value::Number(n) => lua_meta.set(k.as_str(), *n)?,
                _ => lua_meta.set(k.as_str(), "")?,
            }
        }

        let ss = self.shared_state.borrow();
        for (name, tvar) in &ss.variables {
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

    use crate::{rustplay::scripting::Script, value::Value};

    #[test]
    fn test_scripting() {
        let script = r#"

local vars = {
  a = { alias = "one" },
  b = { color = 0xff8040 },
}
set_vars(vars)
template("hello")
        "#;

        let scripting = Script::new(script).unwrap();
        let templ = scripting.get_template();
        assert_eq!(templ, "hello");
        let meta = HashMap::from([("c".to_string(), Value::Text("hey".into()))]);
        let vars = scripting.get_overrides(&meta).unwrap();
        println!("{vars:?}");

        let o = vars.get("b").unwrap();
        assert_eq!(o.color, Some(0xff8040));
    }
}
