use handlebars::Handlebars;
use rngo_sim::spec;
use std::collections::HashMap;
use std::error::Error;
use std::io::Write;
use std::process::{Child, ChildStdin, Command, Stdio};

pub struct SystemDispatch {
    effect_systems: HashMap<String, String>,
    stdinpipes: HashMap<String, ChildStdin>,
    children: HashMap<String, Child>,
    hbs: Handlebars<'static>,
}

impl SystemDispatch {
    pub fn new(spec: &spec::Simulation) -> Result<Self, Box<dyn Error>> {
        let effect_systems: HashMap<String, String> = spec
            .effects
            .iter()
            .filter_map(|(k, v)| v.system.as_ref().map(|s| (k.clone(), s.clone())))
            .collect();

        let system_imports: HashMap<String, spec::SystemImport> = spec
            .systems
            .iter()
            .map(|(k, v)| (k.clone(), v.import.clone()))
            .collect();

        let mut stdinpipes = HashMap::new();
        let mut children = HashMap::new();
        let mut hbs = Handlebars::new();

        for system_key in effect_systems.values() {
            let import = system_imports
                .get(system_key)
                .ok_or_else(|| format!("effect references unknown system: {system_key}"))?;

            match import {
                spec::SystemImport::Stream { command } => {
                    if stdinpipes.contains_key(system_key) {
                        continue;
                    }
                    let mut child = Command::new("sh")
                        .arg("-c")
                        .arg(command)
                        .stdin(Stdio::piped())
                        .spawn()?;
                    let stdin = child.stdin.take().expect("stdin was piped");
                    stdinpipes.insert(system_key.clone(), stdin);
                    children.insert(system_key.clone(), child);
                }
                spec::SystemImport::Exec { command } => {
                    hbs.register_template_string(system_key, command)?;
                }
            }
        }

        Ok(Self {
            effect_systems,
            stdinpipes,
            children,
            hbs,
        })
    }

    pub fn send(
        &mut self,
        effect_key: &str,
        value: &serde_json::Value,
        format: Option<&str>,
    ) -> Result<(), Box<dyn Error>> {
        let system_key = match self.effect_systems.get(effect_key) {
            Some(k) => k.clone(),
            None => return Ok(()),
        };

        if let Some(stdin) = self.stdinpipes.get_mut(&system_key) {
            let output = format
                .map(|f| f.to_string())
                .unwrap_or_else(|| serde_json::to_string(value).unwrap());
            writeln!(stdin, "{output}")?;
        } else if self.hbs.has_template(&system_key) {
            let command = self.hbs.render(&system_key, value)?;
            Command::new("sh").arg("-c").arg(&command).spawn()?.wait()?;
        }

        Ok(())
    }

    pub fn finish(self) -> Result<(), Box<dyn Error>> {
        drop(self.stdinpipes);
        for (_, mut child) in self.children {
            child.wait()?;
        }
        Ok(())
    }
}
