use rhai::FnPtr;
use smartstring::SmartString;
use std::{cell::RefCell, collections::HashMap, error::Error, path::PathBuf, rc::Rc};

/// Script override for a variable in the template string
#[derive(Clone, Debug, Default)]
pub struct TemplateVar {
    color: Option<u32>,
    alias: Option<String>,
    func: Option<FnPtr>,
}

/// Result of overriding
#[derive(Clone, Debug, Default)]
pub struct Override {
    pub color: Option<u32>,
    pub value: Value,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct SharedState {
    template: String,
    variables: HashMap<String, TemplateVar>,
}

use crate::{log, value::Value};

fn to_rhai_map<V: Clone + 'static>(hash_map: &HashMap<String, V>) -> rhai::Map {
    hash_map
        .iter()
        .map(|(k, v)| (SmartString::from(k), rhai::Dynamic::from(v.clone())))
        .collect::<rhai::Map>()
}

pub(crate) struct Scripting {
    engine: rhai::Engine,
    ast: rhai::AST,
    shared_state: Rc<RefCell<SharedState>>,
}

impl Scripting {

    pub fn get_template(&self) -> String {
        self.shared_state.borrow().template.clone()
    }

    pub fn new() -> Result<Self, Box<dyn Error>> {
        let shared_state = Rc::new(RefCell::new(SharedState {
            ..SharedState::default()
        }));

        let mut rhai_engine = rhai::Engine::new();
        rhai_engine
            .register_fn("template", {
                let ss = shared_state.clone();
                move |t: &str| t.clone_into(&mut ss.borrow_mut().template)
            })
            .register_fn("log", move |t: &str| log!("RHAI: {t}"))
            .register_fn("log", move |t: &Value| log!("RHAI: {t}"))
            .register_fn("set_vars", {
                let ss = shared_state.clone();
                move |vars: rhai::Map| {
                    for (key, val) in vars.into_iter() {
                        log!("KEY: {key}");
                        if let Some(m) = val.try_cast::<rhai::Map>() {
                            let mut tvar = TemplateVar {
                                ..Default::default()
                            };
                            log!("Found map");
                            for (key, val) in m.into_iter() {
                                if key == "alias" {
                                    tvar.alias = val.try_cast::<String>();
                                } else if key == "func" {
                                    tvar.func = val.try_cast::<FnPtr>();
                                    log!("Func {:?}", tvar.func);
                                } else if key == "color" {
                                    tvar.color = val.try_cast::<i64>().map(|i| i as u32);
                                    log!("Color {:?}", tvar.color);
                                }
                            }
                            ss.borrow_mut().variables.insert(key.into(), tvar);
                        }
                    }
                }
            });
        rhai_engine.register_type_with_name::<Value>("Value");
        rhai_engine.register_fn("to_string", |v: &mut Value| v.to_string());

        let p = PathBuf::from("init.rhai");
        let ast = if p.is_file() {
            rhai_engine.compile_file(p)?
        } else {
            let script = include_str!("../../init.rhai");
            rhai_engine.compile(script)?
        };
        rhai_engine.run_ast(&ast)?;
        Ok(Scripting {
            engine: rhai_engine,
            ast,
            shared_state: shared_state.clone(),
        })
    }

    pub fn get_overrides(
        &self,
        meta: &HashMap<String, Value>,
    ) -> Result<HashMap<String, Override>, Box<dyn Error>> {
        let mut result = HashMap::new();

        let rhai_map = to_rhai_map(meta);
        let ss = self.shared_state.borrow();
        for (name, tvar) in &ss.variables {
            let o = Override {
                color: tvar.color,
                value: match &tvar.func {
                    Some(func) => {
                        let result =
                            func.call::<String>(&self.engine, &self.ast, (rhai_map.clone(),))?;
                        Value::Text(result)
                    }
                    None => Value::Unknown,
                },
            };
            result.insert(name.into(), o);
        }
        Ok(result)
    }
}
