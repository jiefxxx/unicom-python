use std::{collections::HashMap, path::Path};
use serde_derive::Deserialize;
use walkdir::WalkDir;

use pyo3::{prelude::*, types::{PyDict, PyList}};
use unicom_lib::{node::{NodeConfig, api::{Parameter, ApiMethod}, endpoint::{EndPointKind, EndPoint}}, error::UnicomError};

use super::script::PYTHON_SIGNATURE;

#[derive(Debug, Deserialize)]
pub struct ConfigModel{
    pub name: String,
    pub templates_path: Option<String>,
    pub tags: Option<HashMap<String, String>>,
    pub endpoints: Option<Vec<EndPoint>>,
}

impl ConfigModel {
    fn new() -> ConfigModel{
        let content = std::fs::read_to_string("config.toml").unwrap();
        toml::from_str(&content).unwrap()
    }
}

impl TryInto<NodeConfig> for ConfigModel {
    type Error = UnicomError;

    fn try_into(self) -> Result<NodeConfig, Self::Error> {
        let mut config = NodeConfig::new(&self.name);
            if self.templates_path.is_some(){
                for entry in WalkDir::new(self.templates_path.unwrap())
                        .follow_links(true)
                        .into_iter()
                        .filter_map(|e| e.ok()) {
                    
                    println!("{:?}", entry);
                    
                    if !entry.file_type().is_file(){
                        continue
                    }
                    
                    let mut data :Vec<&str> = entry.path().to_str().unwrap().split("/").collect();
                    data.remove(0);

                    let terra_path = Path::new(&self.name).join(data.join("/"));
                    let absolute_path = entry.path().canonicalize().unwrap();

                    println!("{} _ {}", terra_path.to_str().unwrap(), absolute_path.to_str().unwrap());

                    config.add_template(absolute_path.to_str().unwrap(), terra_path.to_str().unwrap());
                    
                }
            }
            if self.tags.is_some(){
                config.tags = self.tags.unwrap();
            }
            if self.endpoints.is_some(){
                for endpoint in self.endpoints.unwrap(){
                    let mut n_endpoint = endpoint.clone();
                    if let Some(endpoint_kind) = match endpoint.kind {
                        EndPointKind::Static { path } => {
                            Some(EndPointKind::Static { path: Path::new(&path).canonicalize().unwrap().to_str().unwrap().to_string() })
                        },
                        _ => None
                    }{
                        n_endpoint.kind = endpoint_kind;
                    }
                    config.endpoints.push(n_endpoint);
                }
            }
            Ok(config)
    }
}


#[derive(Debug, Clone)]
#[pyclass]
pub struct PythonConfig{
    pub config: NodeConfig,
    pub api_objects: Vec<PyObject>,
}


impl PythonConfig{
    pub fn new() -> PythonConfig{
        let config = ConfigModel::new();
        PythonConfig { 
            config: config.try_into().unwrap(), 
            api_objects: Vec::new(),
        }
    }
}

#[pymethods]
impl PythonConfig{

    pub fn add_api(&mut self, name: String, object: PyObject) -> PyResult<String>{
        let mut methodes = Vec::new();
        let list_methodes = vec!["GET", "POST", "PUT", "DELETE"];
        Python::with_gil(|py| -> PyResult<()>{
            for s_methode in list_methodes{
                if let Ok(methode) = object.getattr(py, s_methode){
                    let data = PYTHON_SIGNATURE.call1(py, (methode,))?;
                    let list: &PyList = data.extract(py)?;
                    let mut parameters = Vec::new();
                    for dict in list{
                        let dict: &PyDict = dict.extract().unwrap();
                        let p_name = dict.get_item("name").unwrap().extract()?;
                        let p_kind: &str = dict.get_item("kind").unwrap().extract()?;
                        let p_mandatory = dict.get_item("mandatory").unwrap().extract()?;
                        parameters.push(Parameter::new(p_name, p_kind.into(), p_mandatory));
                    }
                    methodes.push(ApiMethod::new(s_methode.into(), parameters))
    
                }
            }

            Ok(())
            
        })?;

        let id = self.api_objects.len() as u64;
        self.config.add_api(id, &name, methodes);
        self.api_objects.push(object);

        Ok(name)
    }
}